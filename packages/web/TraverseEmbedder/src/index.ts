/**
 * Public Traverse platform embedder SDK for Web/TypeScript clients.
 *
 * This package is the Web row of spec `068-public-platform-embedder-packages`:
 * a versioned public package implementing the `embedder-api/1.0.0` operations
 * (spec `057-embeddable-runtime-host`) against an application-owned bundle,
 * with no production dependency on `traverse-cli serve`.
 *
 * The event envelope, deterministic identifier scheme (`sess-*`, `req-*`,
 * `evt-*`, `inst-*`), stable error codes, compatible-capability lifecycle,
 * and shutdown semantics are identical to the Rust `traverse-embedder`
 * package so every platform observes the same boundary (spec 057 FR-003).
 */

/** Implemented embedder API version (spec 057 IDL `$id` suffix). */
export const EMBEDDER_API_VERSION = "1.0.0";

/** Conformance suite revision this package certifies against (spec 057). */
export const EMBEDDER_CONFORMANCE_VERSION = "1.0.0";

/** Application bundle manifest `schema_version` values this package accepts. */
export const SUPPORTED_BUNDLE_SCHEMA_VERSIONS: readonly string[] = ["1.0.0"];

const EVENT_SCHEMA_VERSION = "1.0.0";
const PACKAGE_NAME = "traverse-embedder-web";
const PACKAGE_VERSION = "0.7.0";

/** Stable embedder-boundary error codes (wire-identical to the Rust SDK). */
export type EmbedderErrorCode =
  | "bundle_load_failed"
  | "unsupported_bundle_schema"
  | "runtime_stopped"
  | "target_not_found"
  | "compatible_lifecycle_required"
  | "capability_not_compatible"
  | "platform_not_supported"
  | "instance_not_found"
  | "instance_not_running";

/** A structured embedder-boundary error. */
export interface EmbedderError {
  readonly code: EmbedderErrorCode;
  readonly message: string;
}

/** `runtime.submit` output. */
export interface SubmitOutcome {
  readonly sessionId: string | null;
  readonly status: "accepted" | "rejected";
  readonly error: EmbedderError | null;
}

/** `compatible.start` output. */
export interface CompatibleStartOutcome {
  readonly instanceId: string | null;
  readonly status: "started" | "error";
  readonly error: EmbedderError | null;
}

/** `compatible.stop` / `compatible.kill` output. */
export interface CompatibleLifecycleOutcome {
  readonly status: "stopped" | "killed" | "error";
  readonly error: EmbedderError | null;
}

/** `runtime.shutdown` output (always stopped; idempotent). */
export interface ShutdownOutcome {
  readonly killedInstances: number;
}

/** JSON value type for wire payloads. */
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

/**
 * Ordered embedder event (JSON wire format, spec 057). The envelope is
 * byte-identical in field naming to the Rust SDK's `embedder_event`.
 */
export interface EmbedderEvent {
  readonly kind: "embedder_event";
  readonly schema_version: string;
  readonly embedder_api_version: string;
  readonly event_id: string;
  readonly sequence: number;
  readonly event_type:
    | "state_changed"
    | "capability_invoked"
    | "capability_result"
    | "error";
  readonly workspace_id: string;
  readonly app_id: string;
  readonly session_id: string | null;
  readonly data: JsonValue;
}

/** Ordered, synchronous event subscriber. */
export type EventCallback = (event: EmbedderEvent) => void;

/**
 * The uniform `embedder-api/1.0.0` operation surface (spec 057 FR-003).
 *
 * `EmbedderTestDouble` is the deterministic in-memory implementation
 * required by spec 068 FR-006; the production runtime-WASM embedder
 * implements the same boundary.
 */
export interface TraverseEmbedderApi {
  /** `runtime.submit`: execute a bundled workflow or WASM capability. */
  submit(targetId: string, input: JsonValue): SubmitOutcome;
  /**
   * `runtime.subscribe`: register an ordered event callback. Previously
   * emitted events are replayed to the new subscriber first, so late
   * subscribers observe the identical ordered stream.
   */
  subscribe(callback: EventCallback): void;
  /** `compatible.start`: start a compatible-mode capability instance. */
  startCompatible(capabilityId: string, input: JsonValue): CompatibleStartOutcome;
  /**
   * `compatible.stop`: gracefully stop one instance (`instanceId`) or every
   * running instance of the capability (`null`).
   */
  stopCompatible(
    capabilityId: string,
    instanceId?: string | null,
  ): CompatibleLifecycleOutcome;
  /**
   * `compatible.kill`: force-terminate one instance (`instanceId`) or every
   * running instance of the capability (`null`).
   */
  killCompatible(
    capabilityId: string,
    instanceId?: string | null,
  ): CompatibleLifecycleOutcome;
  /**
   * `runtime.shutdown`: kill running compatible instances and stop accepting
   * operations. Idempotent.
   */
  shutdown(): ShutdownOutcome;
  /**
   * Release evidence connecting this embedder to its package version,
   * runtime, conformance version, and bundle digests (spec 068 FR-008,
   * NFR-002).
   */
  releaseEvidence(): JsonValue;
}

function embedderError(code: EmbedderErrorCode, message: string): EmbedderError {
  return { code, message };
}

function errorValue(error: EmbedderError): JsonValue {
  return { code: error.code, message: error.message };
}

function paddedId(prefix: string, counter: number): string {
  return `${prefix}-${String(counter).padStart(8, "0")}`;
}

function runtimeStoppedError(): EmbedderError {
  return embedderError(
    "runtime_stopped",
    "the embedded runtime was shut down and accepts no further operations",
  );
}

type InstanceState = "started" | "stopped" | "killed";

interface CompatibleInstance {
  readonly capabilityId: string;
  state: InstanceState;
}

/**
 * Shared deterministic embedder state: identity, counters, subscribers,
 * event history, and the compatible-capability lifecycle table. Identical
 * semantics to the Rust SDK's internal core.
 */
class EmbedderCore {
  readonly workspaceId: string;
  readonly appId: string;
  readonly appVersion: string;
  readonly platform: string;
  readonly compatibleTargets: Map<string, readonly string[]>;
  private readonly instances = new Map<string, CompatibleInstance>();
  private readonly subscribers: EventCallback[] = [];
  private readonly history: EmbedderEvent[] = [];
  private nextEvent = 0;
  private nextSession = 0;
  private nextRequest = 0;
  private nextInstance = 0;
  stopped = false;

  constructor(
    workspaceId: string,
    appId: string,
    appVersion: string,
    platform: string,
    compatibleTargets: Map<string, readonly string[]>,
  ) {
    this.workspaceId = workspaceId;
    this.appId = appId;
    this.appVersion = appVersion;
    this.platform = platform;
    this.compatibleTargets = compatibleTargets;
  }

  nextSessionId(): string {
    this.nextSession += 1;
    return paddedId("sess", this.nextSession);
  }

  nextRequestId(): string {
    this.nextRequest += 1;
    return paddedId("req", this.nextRequest);
  }

  private nextInstanceId(): string {
    this.nextInstance += 1;
    return paddedId("inst", this.nextInstance);
  }

  emit(
    eventType: EmbedderEvent["event_type"],
    sessionId: string | null,
    data: JsonValue,
  ): void {
    this.nextEvent += 1;
    const event: EmbedderEvent = {
      kind: "embedder_event",
      schema_version: EVENT_SCHEMA_VERSION,
      embedder_api_version: EMBEDDER_API_VERSION,
      event_id: paddedId("evt", this.nextEvent),
      sequence: this.nextEvent,
      event_type: eventType,
      workspace_id: this.workspaceId,
      app_id: this.appId,
      session_id: sessionId,
      data,
    };
    for (const subscriber of this.subscribers) {
      subscriber(event);
    }
    this.history.push(event);
  }

  subscribe(callback: EventCallback): void {
    for (const event of this.history) {
      callback(event);
    }
    this.subscribers.push(callback);
  }

  emitErrorEvent(
    sessionId: string | null,
    error: EmbedderError,
    data: { [key: string]: JsonValue },
  ): void {
    this.emit("error", sessionId, { ...data, error: errorValue(error) });
  }

  rejectedSubmit(targetId: string, error: EmbedderError): SubmitOutcome {
    this.emitErrorEvent(null, error, { target_id: targetId });
    return { sessionId: null, status: "rejected", error };
  }

  startCompatible(capabilityId: string, input: JsonValue): CompatibleStartOutcome {
    let error: EmbedderError | null = null;
    if (this.stopped) {
      error = runtimeStoppedError();
    } else {
      const platforms = this.compatibleTargets.get(capabilityId);
      if (platforms === undefined) {
        error = embedderError(
          "capability_not_compatible",
          `capability '${capabilityId}' is not a compatible-mode capability in this bundle`,
        );
      } else if (!platforms.includes(this.platform)) {
        error = embedderError(
          "platform_not_supported",
          `capability '${capabilityId}' permits platforms [${platforms.join(", ")}] ` +
            `but this embedder runs on '${this.platform}'`,
        );
      }
    }
    if (error !== null) {
      this.emitErrorEvent(null, error, { capability_id: capabilityId });
      return { instanceId: null, status: "error", error };
    }

    const instanceId = this.nextInstanceId();
    this.instances.set(instanceId, { capabilityId, state: "started" });
    this.emit("state_changed", null, {
      capability_id: capabilityId,
      instance_id: instanceId,
      state: "started",
      previous_state: null,
      input,
    });
    return { instanceId, status: "started", error: null };
  }

  transitionCompatible(
    capabilityId: string,
    instanceId: string | null,
    targetState: "stopped" | "killed",
  ): CompatibleLifecycleOutcome {
    if (this.stopped) {
      const error = runtimeStoppedError();
      this.emitErrorEvent(null, error, { capability_id: capabilityId });
      return { status: "error", error };
    }

    let selected: string[];
    if (instanceId !== null) {
      const instance = this.instances.get(instanceId);
      if (instance === undefined || instance.capabilityId !== capabilityId) {
        const error = embedderError(
          "instance_not_found",
          `no instance '${instanceId}' exists for capability '${capabilityId}'`,
        );
        this.emitErrorEvent(null, error, {
          capability_id: capabilityId,
          instance_id: instanceId,
        });
        return { status: "error", error };
      }
      if (instance.state !== "started") {
        const error = embedderError(
          "instance_not_running",
          `instance '${instanceId}' of capability '${capabilityId}' is not running`,
        );
        this.emitErrorEvent(null, error, {
          capability_id: capabilityId,
          instance_id: instanceId,
        });
        return { status: "error", error };
      }
      selected = [instanceId];
    } else {
      selected = [...this.instances.entries()]
        .filter(
          ([, instance]) =>
            instance.capabilityId === capabilityId && instance.state === "started",
        )
        .map(([id]) => id);
    }

    if (selected.length === 0) {
      const error = embedderError(
        "instance_not_running",
        `capability '${capabilityId}' has no running instances`,
      );
      this.emitErrorEvent(null, error, { capability_id: capabilityId });
      return { status: "error", error };
    }

    for (const id of selected) {
      this.setInstanceState(id, targetState);
    }
    return { status: targetState, error: null };
  }

  private setInstanceState(instanceId: string, targetState: InstanceState): void {
    const instance = this.instances.get(instanceId);
    if (instance === undefined) {
      return;
    }
    const previous = instance.state;
    instance.state = targetState;
    this.emit("state_changed", null, {
      capability_id: instance.capabilityId,
      instance_id: instanceId,
      state: targetState,
      previous_state: previous,
    });
  }

  shutdown(): ShutdownOutcome {
    if (this.stopped) {
      return { killedInstances: 0 };
    }
    const running = [...this.instances.entries()]
      .filter(([, instance]) => instance.state === "started")
      .map(([id]) => id);
    for (const id of running) {
      this.setInstanceState(id, "killed");
    }
    this.stopped = true;
    return { killedInstances: running.length };
  }

  evidence(runtimeImplementation: string, wasmComponents: JsonValue): JsonValue {
    return {
      kind: "embedder_release_evidence",
      schema_version: EVENT_SCHEMA_VERSION,
      package: { name: PACKAGE_NAME, version: PACKAGE_VERSION },
      embedder_api_version: EMBEDDER_API_VERSION,
      conformance_version: EMBEDDER_CONFORMANCE_VERSION,
      runtime: { implementation: runtimeImplementation },
      supported_bundle_schema_versions: [...SUPPORTED_BUNDLE_SCHEMA_VERSIONS],
      bundle: {
        app_id: this.appId,
        app_version: this.appVersion,
        wasm_components: wasmComponents,
      },
      workspace_id: this.workspaceId,
      platform: this.platform,
    };
  }
}

/** Configuration for the deterministic test double. */
export interface EmbedderTestDoubleConfig {
  readonly workspaceId?: string;
  readonly appId?: string;
  readonly appVersion?: string;
  readonly platform?: string;
}

type ScriptedResult =
  | { readonly kind: "output"; readonly output: JsonValue }
  | { readonly kind: "error"; readonly code: string; readonly message: string };

/**
 * Deterministic in-memory test double implementing the same public boundary
 * as the production embedder (spec 068 FR-006). It shares the event
 * envelope, identifier scheme, compatible lifecycle, and shutdown semantics;
 * only capability execution is replaced with scripted results. It contains
 * no business logic.
 */
export class EmbedderTestDouble implements TraverseEmbedderApi {
  private readonly core: EmbedderCore;
  private readonly scripted = new Map<string, ScriptedResult>();

  constructor(config: EmbedderTestDoubleConfig = {}) {
    this.core = new EmbedderCore(
      config.workspaceId ?? "local-default",
      config.appId ?? "test-app",
      config.appVersion ?? "1.0.0",
      config.platform ?? "web",
      new Map(),
    );
  }

  /** Scripts `submit(targetId, _)` to succeed with `output`. */
  withTargetOutput(targetId: string, output: JsonValue): this {
    this.scripted.set(targetId, { kind: "output", output });
    return this;
  }

  /** Scripts `submit(targetId, _)` to fail with a runtime-shaped error. */
  withTargetError(targetId: string, code: string, message: string): this {
    this.scripted.set(targetId, { kind: "error", code, message });
    return this;
  }

  /** Declares a compatible-mode capability with a platform allowlist. */
  withCompatibleTarget(capabilityId: string, platforms: readonly string[]): this {
    this.core.compatibleTargets.set(capabilityId, platforms);
    return this;
  }

  submit(targetId: string, input: JsonValue): SubmitOutcome {
    void input;
    if (this.core.stopped) {
      return this.core.rejectedSubmit(targetId, runtimeStoppedError());
    }
    const result = this.scripted.get(targetId);
    if (result === undefined) {
      return this.core.rejectedSubmit(
        targetId,
        embedderError(
          "target_not_found",
          `'${targetId}' is neither a bundled workflow nor a bundled capability`,
        ),
      );
    }

    const sessionId = this.core.nextSessionId();
    const requestId = this.core.nextRequestId();
    const executionId = `exec_${requestId}`;
    this.core.emit("capability_invoked", sessionId, {
      execution_id: executionId,
      capability_id: targetId,
      capability_version: "1.0.0",
    });
    if (result.kind === "output") {
      this.core.emit("capability_result", sessionId, {
        execution_id: executionId,
        capability_id: targetId,
        status: "completed",
        output: result.output,
      });
    } else {
      this.core.emit("error", sessionId, {
        execution_id: executionId,
        capability_id: targetId,
        status: "error",
        error: { code: result.code, message: result.message, details: {} },
      });
    }
    return { sessionId, status: "accepted", error: null };
  }

  subscribe(callback: EventCallback): void {
    this.core.subscribe(callback);
  }

  startCompatible(capabilityId: string, input: JsonValue): CompatibleStartOutcome {
    return this.core.startCompatible(capabilityId, input);
  }

  stopCompatible(
    capabilityId: string,
    instanceId: string | null = null,
  ): CompatibleLifecycleOutcome {
    return this.core.transitionCompatible(capabilityId, instanceId, "stopped");
  }

  killCompatible(
    capabilityId: string,
    instanceId: string | null = null,
  ): CompatibleLifecycleOutcome {
    return this.core.transitionCompatible(capabilityId, instanceId, "killed");
  }

  shutdown(): ShutdownOutcome {
    return this.core.shutdown();
  }

  releaseEvidence(): JsonValue {
    return this.core.evidence("test-double", []);
  }
}

/** One bundled component reference parsed from the app manifest. */
export interface BundleComponentSummary {
  readonly componentId: string;
  readonly version: string;
  readonly digest: string;
  readonly manifestPath: string;
}

/** Deterministic bundle compatibility summary (spec 068 NFR-001). */
export interface BundleCompatibility {
  readonly appId: string;
  readonly appVersion: string;
  readonly schemaVersion: string;
  readonly components: readonly BundleComponentSummary[];
  readonly workflowIds: readonly string[];
}

/** Thrown when a bundle is rejected at the embedder boundary. */
export class BundleRejectedError extends Error {
  readonly embedderError: EmbedderError;

  constructor(error: EmbedderError) {
    super(`${error.code}: ${error.message}`);
    this.name = "BundleRejectedError";
    this.embedderError = error;
  }
}

function asRecord(value: JsonValue | undefined): { [key: string]: JsonValue } | null {
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    return value;
  }
  return null;
}

function requiredString(
  record: { [key: string]: JsonValue },
  key: string,
  context: string,
): string {
  const value = record[key];
  if (typeof value !== "string" || value.trim() === "") {
    throw new BundleRejectedError(
      embedderError(
        "bundle_load_failed",
        `${context} requires a non-empty string '${key}'`,
      ),
    );
  }
  return value;
}

const SHA256_DIGEST_PATTERN = /^sha256:[0-9a-f]{64}$/;

/**
 * Parses and deterministically validates an application bundle manifest
 * (spec `044-application-bundle-manifest`) for embedder compatibility:
 * schema version support, component identity, and sha-256 digest metadata.
 * Rejection never falls back to a sidecar (spec 068 NFR-001).
 *
 * @throws {BundleRejectedError} with a stable `EmbedderErrorCode`.
 */
export function validateBundleCompatibility(
  appManifest: string | JsonValue,
): BundleCompatibility {
  let parsed: JsonValue;
  if (typeof appManifest === "string") {
    try {
      parsed = JSON.parse(appManifest) as JsonValue;
    } catch (error) {
      throw new BundleRejectedError(
        embedderError(
          "bundle_load_failed",
          `application bundle manifest is not valid JSON: ${String(error)}`,
        ),
      );
    }
  } else {
    parsed = appManifest;
  }

  const manifest = asRecord(parsed);
  if (manifest === null) {
    throw new BundleRejectedError(
      embedderError(
        "bundle_load_failed",
        "application bundle manifest must be a JSON object",
      ),
    );
  }

  const appId = requiredString(manifest, "app_id", "application bundle manifest");
  const appVersion = requiredString(manifest, "version", "application bundle manifest");
  const schemaVersion = requiredString(
    manifest,
    "schema_version",
    "application bundle manifest",
  );
  if (!SUPPORTED_BUNDLE_SCHEMA_VERSIONS.includes(schemaVersion)) {
    throw new BundleRejectedError(
      embedderError(
        "unsupported_bundle_schema",
        `bundle declares schema_version '${schemaVersion}' but this package supports ` +
          `[${SUPPORTED_BUNDLE_SCHEMA_VERSIONS.join(", ")}]; no sidecar fallback is attempted`,
      ),
    );
  }

  const componentsValue = manifest["components"];
  if (!Array.isArray(componentsValue)) {
    throw new BundleRejectedError(
      embedderError(
        "bundle_load_failed",
        "application bundle manifest requires a 'components' array",
      ),
    );
  }
  const components: BundleComponentSummary[] = componentsValue.map((entry, index) => {
    const component = asRecord(entry);
    if (component === null) {
      throw new BundleRejectedError(
        embedderError(
          "bundle_load_failed",
          `components[${index}] must be a JSON object`,
        ),
      );
    }
    const context = `components[${index}]`;
    const digest = requiredString(component, "digest", context);
    if (!SHA256_DIGEST_PATTERN.test(digest)) {
      throw new BundleRejectedError(
        embedderError(
          "bundle_load_failed",
          `${context} declares invalid digest metadata '${digest}'; ` +
            "expected sha256:<64 hex characters>",
        ),
      );
    }
    return {
      componentId: requiredString(component, "component_id", context),
      version: requiredString(component, "version", context),
      digest,
      manifestPath: requiredString(component, "manifest_path", context),
    };
  });

  const workflowsValue = manifest["workflows"];
  const workflowIds: string[] = [];
  if (Array.isArray(workflowsValue)) {
    for (const [index, entry] of workflowsValue.entries()) {
      const workflow = asRecord(entry);
      if (workflow === null) {
        throw new BundleRejectedError(
          embedderError(
            "bundle_load_failed",
            `workflows[${index}] must be a JSON object`,
          ),
        );
      }
      workflowIds.push(requiredString(workflow, "workflow_id", `workflows[${index}]`));
    }
  }

  return { appId, appVersion, schemaVersion, components, workflowIds };
}

/**
 * Verifies bundled artifact bytes against declared sha-256 digest metadata
 * using WebCrypto (browser) or the Node.js webcrypto implementation.
 *
 * @throws {BundleRejectedError} with `bundle_load_failed` on mismatch.
 */
export async function verifyArtifactDigest(
  bytes: Uint8Array,
  declaredDigest: string,
  artifactLabel: string,
): Promise<void> {
  if (!SHA256_DIGEST_PATTERN.test(declaredDigest)) {
    throw new BundleRejectedError(
      embedderError(
        "bundle_load_failed",
        `${artifactLabel} declares invalid digest metadata '${declaredDigest}'`,
      ),
    );
  }
  const digestBytes = await crypto.subtle.digest(
    "SHA-256",
    bytes.slice().buffer,
  );
  const actual = [...new Uint8Array(digestBytes)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  const expected = declaredDigest.slice("sha256:".length);
  if (actual !== expected) {
    throw new BundleRejectedError(
      embedderError(
        "bundle_load_failed",
        `${artifactLabel} digest mismatch: manifest declares sha256:${expected} ` +
          `but the bundled artifact hashes to sha256:${actual}; ` +
          "no sidecar fallback is attempted",
      ),
    );
  }
}

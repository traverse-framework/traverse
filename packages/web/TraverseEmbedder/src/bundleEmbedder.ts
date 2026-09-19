/**
 * Production embedder: loads an application-owned bundle and executes
 * bundled WASM capabilities through the shared `runtime.wasm` orchestrator
 * (spec `1402` FR-006/FR-007), loaded from the bundle via `BundleLoader` —
 * not embedded in the npm package (spec `068` FR-002).
 *
 * Workflow execution supports linear, `direct`-triggered pipelines only
 * (the shape used by every bundled example workflow today: `analyze` ->
 * `recommend`, single-node `process`). Event-driven / conditional workflow
 * edges (specs 052/053) are out of scope for this package slice and are
 * rejected deterministically at `init` rather than silently mis-executed.
 */
import { EmbedderCore } from "./core.js";
import {
  BundleRejectedError,
  asRecord,
  optionalString,
  requiredString,
  validateBundleCompatibility,
  verifyArtifactDigest,
  SHA256_DIGEST_PATTERN,
} from "./bundleValidation.js";
import type { BundleLoader } from "./bundleLoader.js";
import { IndexedDbDataStore } from "./indexedDbDataStore.js";
import {
  RuntimeWasmHost,
  RuntimeWasmHostError,
  mapRuntimeWasmEvents,
  mapServiceType,
  parseDeclaredEmits,
} from "./runtimeWasmHost.js";
import type { RuntimeWasmEmitRef, RuntimeWasmJson } from "./runtimeWasmHost.js";
import { attestStatefulBrowserActivation } from "./statefulBrowserActivation.js";
import { EMBEDDED_TRACE_API_VERSION, embedderError, runtimeStoppedError } from "./types.js";
import type {
  CompatibleLifecycleOutcome,
  CompatibleStartOutcome,
  EmbeddedTraceApi,
  EmbeddedTraceApiError,
  EmbeddedTraceDetail,
  EmbeddedTracePage,
  EventCallback,
  JsonValue,
  ShutdownOutcome,
  AppCommandEnvelope,
  EmbedderEvent,
  SubmitOutcome,
  TraverseEmbedderApi,
} from "./types.js";

const DEFAULT_RUNTIME_WASM_PATH = "runtime/runtime.wasm";

/** Configuration for `BundleEmbedder.init` (`runtime.init` input). */
export interface BundleEmbedderConfig {
  /** Path or URL to the application bundle's `app.manifest.json`. */
  readonly manifestPath: string;
  /** Loader used to fetch bundle files (browser: `FetchBundleLoader`). */
  readonly loader: BundleLoader;
  /** Workspace identity recorded on events. Defaults to `local-default`. */
  readonly workspaceId?: string;
  /** Platform identity checked against compatible-capability allowlists. */
  readonly platform?: string;
  /**
   * Bundle-relative path to `runtime.wasm` (default `runtime/runtime.wasm`).
   * Digest is read from the companion `<path>.sha256` sidecar.
   */
  readonly runtimeWasmPath?: string;
}

interface WasmTarget {
  readonly capabilityId: string;
  readonly capabilityVersion: string;
  readonly digest: string;
  /** Nested capability artifact bytes (executed inside `runtime.wasm`). */
  readonly wasmBytes: Uint8Array;
  /** Optional contract `service_type` from the component manifest (Spec 132). */
  readonly serviceType: string | null;
  /** Declared contract `emits` entries forwarded to `runtime.wasm` init (spec 098). */
  readonly emits: readonly RuntimeWasmEmitRef[];
}

interface WorkflowNodeSpec {
  readonly nodeId: string;
  readonly capabilityId: string;
  readonly capabilityVersion: string;
  readonly fromWorkflowInput: readonly string[];
  readonly toWorkflowState: readonly string[];
  readonly publishToStateAs: string | null;
}

interface WorkflowTarget {
  readonly version: string;
  readonly nodes: ReadonlyMap<string, WorkflowNodeSpec>;
  readonly nextByFrom: ReadonlyMap<string, string>;
  readonly startNode: string;
  readonly outputProjection: readonly string[];
}

interface WasmExecutionResult {
  readonly ok: boolean;
  readonly output: JsonValue | null;
  readonly code: string;
  readonly message: string;
}

type WorkflowStepStatus = "completed" | "failed";

interface WorkflowStepRecord {
  readonly stepIndex: number;
  readonly nodeId: string;
  readonly capabilityId: string;
  readonly capabilityVersion: string;
  readonly status: WorkflowStepStatus;
}

function loadFailure(message: string): BundleRejectedError {
  return new BundleRejectedError(embedderError("bundle_load_failed", message));
}

/** Copies `bytes` into a fresh, non-shared `ArrayBuffer` for WebAssembly APIs. */
function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

function stringArray(value: JsonValue | undefined, context: string): string[] {
  if (value === undefined) {
    return [];
  }
  if (!Array.isArray(value)) {
    throw loadFailure(`${context} must be an array`);
  }
  return value.map((entry, index) => {
    if (typeof entry !== "string") {
      throw loadFailure(`${context}[${index}] must be a string`);
    }
    return entry;
  });
}

function initialWorkflowState(input: JsonValue): Record<string, JsonValue> {
  const record = asRecord(input);
  return record !== null ? { ...record } : { input };
}

function buildNodeInput(
  state: Record<string, JsonValue>,
  keys: readonly string[],
): JsonValue {
  const input: Record<string, JsonValue> = {};
  for (const key of keys) {
    const value = state[key];
    if (value !== undefined) {
      input[key] = value;
    }
  }
  return input;
}

function applyNodeOutput(
  state: Record<string, JsonValue>,
  node: WorkflowNodeSpec,
  output: JsonValue,
): void {
  const record = asRecord(output);
  if (record === null) {
    return;
  }
  for (const key of node.toWorkflowState) {
    const value = record[key];
    if (value !== undefined) {
      state[key] = value;
    }
  }
  if (node.publishToStateAs !== null) {
    state[node.publishToStateAs] = output;
  }
}

function projectWorkflowOutput(
  state: Record<string, JsonValue>,
  projection: readonly string[],
): JsonValue {
  if (projection.length === 0) {
    return { ...state };
  }
  const projected: Record<string, JsonValue> = {};
  for (const key of projection) {
    const value = state[key];
    if (value !== undefined) {
      projected[key] = value;
    }
  }
  return projected;
}

function parseRuntimeDigestText(text: string, digestPath: string): string {
  const trimmed = text.trim();
  const firstToken = trimmed.split(/\s+/)[0] ?? "";
  if (!SHA256_DIGEST_PATTERN.test(firstToken)) {
    throw loadFailure(
      `runtime digest file '${digestPath}' must contain a sha256:<hex> digest; got '${trimmed}'`,
    );
  }
  return firstToken.toLowerCase();
}

function classifyRuntimeFailure(message: string): { code: string; message: string } {
  const lower = message.toLowerCase();
  if (lower.includes("not valid json") || lower.includes("deserialization")) {
    return { code: "output_deserialization_failed", message };
  }
  if (
    lower.includes("unauthorized") ||
    lower.includes("unknown import") ||
    lower.includes("import ") ||
    lower.includes("link ")
  ) {
    return { code: "constraint_violated", message };
  }
  return { code: "execution_failed", message };
}

function asJsonValue(value: RuntimeWasmJson): JsonValue {
  return value as JsonValue;
}



function mapAppLifecycleEventType(
  rawType: string,
): EmbedderEvent["event_type"] {
  switch (rawType) {
    case "state_changed":
    case "capability_invoked":
    case "capability_result":
    case "capability_event":
    case "capability_succeeded":
    case "capability_failed":
    case "host_connector_succeeded":
    case "host_connector_failed":
    case "host_connector_cancelled":
    case "host_connector_timeout":
    case "error":
    case "heartbeat":
      return rawType;
    default:
      return "error";
  }
}

function primaryInvokeWasmTarget(
  stateMachine: JsonValue,
  wasmTargets: ReadonlyMap<string, WasmTarget>,
): WasmTarget | null {
  const record = asRecord(stateMachine);
  const states = record?.["states"];
  if (Array.isArray(states)) {
    for (const entry of states) {
      const state = asRecord(entry);
      const invoke = state ? asRecord(state["invoke"]) : null;
      const capabilityId =
        invoke && typeof invoke["capability_id"] === "string"
          ? invoke["capability_id"]
          : null;
      if (capabilityId !== null) {
        const target = wasmTargets.get(capabilityId);
        if (target !== undefined) {
          return target;
        }
      }
    }
  }
  for (const target of wasmTargets.values()) {
    return target;
  }
  return null;
}

export class BundleEmbedder implements TraverseEmbedderApi, EmbeddedTraceApi {
  private readonly core: EmbedderCore;
  private readonly runtimeModule: WebAssembly.Module;
  private readonly runtimeDigest: string;
  private readonly wasmTargets: ReadonlyMap<string, WasmTarget>;
  private readonly workflowTargets: ReadonlyMap<string, WorkflowTarget>;
  private readonly wasmComponentEvidence: readonly JsonValue[];
  private readonly stateMachine: JsonValue | null;
  private indexedDbDataStore: IndexedDbDataStore | null = null;
  /** Long-lived Spec 139 app orchestrator instance (process-local sessions). */
  private appHost: RuntimeWasmHost | null = null;
  /** Host-owned monotonic deadline handles for outstanding Spec 139 waits. */
  private readonly appDeadlineTimers = new Set<ReturnType<typeof setTimeout>>();

  private constructor(
    core: EmbedderCore,
    runtimeModule: WebAssembly.Module,
    runtimeDigest: string,
    wasmTargets: ReadonlyMap<string, WasmTarget>,
    workflowTargets: ReadonlyMap<string, WorkflowTarget>,
    wasmComponentEvidence: readonly JsonValue[],
    stateMachine: JsonValue | null,
  ) {
    this.core = core;
    this.runtimeModule = runtimeModule;
    this.runtimeDigest = runtimeDigest;
    this.wasmTargets = wasmTargets;
    this.workflowTargets = workflowTargets;
    this.wasmComponentEvidence = wasmComponentEvidence;
    this.stateMachine = stateMachine;
  }

  /**
   * Binds a Spec `085` IndexedDB DataStore for Stateful Browser activation
   * (Spec `132`). Pass `null` to clear. Attestation inspects this handle —
   * an honor-system flag is not accepted.
   */
  bindIndexedDbDataStore(store: IndexedDbDataStore | null): void {
    this.indexedDbDataStore = store;
  }

  /**
   * `runtime.init`: load and digest-verify the application bundle plus
   * `runtime.wasm`. Rejects deterministically with a `BundleRejectedError`
   * and never falls back to a sidecar (spec 068 NFR-001). Host-ABI import
   * validation for nested capabilities is owned by `runtime.wasm` (FR-007).
   */
  static async init(config: BundleEmbedderConfig): Promise<BundleEmbedder> {
    const { manifestPath, loader } = config;
    let manifestText: string;
    try {
      manifestText = await loader.loadText(manifestPath);
    } catch (error) {
      throw loadFailure(`failed to load application bundle manifest: ${String(error)}`);
    }
    const summary = validateBundleCompatibility(manifestText);

    const runtimeRelPath = config.runtimeWasmPath ?? DEFAULT_RUNTIME_WASM_PATH;
    const runtimePath = loader.resolve(manifestPath, runtimeRelPath);
    const runtimeDigestPath = `${runtimePath}.sha256`;
    let runtimeBytes: Uint8Array;
    try {
      runtimeBytes = await loader.loadBytes(runtimePath);
    } catch (error) {
      throw loadFailure(`failed to load runtime.wasm '${runtimePath}': ${String(error)}`);
    }
    let runtimeDigestText: string;
    try {
      runtimeDigestText = await loader.loadText(runtimeDigestPath);
    } catch (error) {
      throw loadFailure(
        `failed to load runtime.wasm digest '${runtimeDigestPath}': ${String(error)}`,
      );
    }
    const runtimeDigest = parseRuntimeDigestText(runtimeDigestText, runtimeDigestPath);
    await verifyArtifactDigest(runtimeBytes, runtimeDigest, "runtime.wasm artifact");
    let runtimeModule: WebAssembly.Module;
    try {
      runtimeModule = await WebAssembly.compile(toArrayBuffer(runtimeBytes));
    } catch (error) {
      throw loadFailure(`runtime.wasm failed to compile: ${String(error)}`);
    }

    const wasmTargets = new Map<string, WasmTarget>();
    const compatibleTargets = new Map<string, readonly string[]>();
    const wasmComponentEvidence: JsonValue[] = [];

    for (const component of summary.components) {
      const componentManifestPath = loader.resolve(manifestPath, component.manifestPath);
      let componentManifestText: string;
      try {
        componentManifestText = await loader.loadText(componentManifestPath);
      } catch (error) {
        throw loadFailure(
          `failed to load component manifest '${componentManifestPath}': ${String(error)}`,
        );
      }
      let parsed: JsonValue;
      try {
        parsed = JSON.parse(componentManifestText) as JsonValue;
      } catch (error) {
        throw loadFailure(
          `component manifest '${componentManifestPath}' is not valid JSON: ${String(error)}`,
        );
      }
      const record = asRecord(parsed);
      if (record === null) {
        throw loadFailure(`component manifest '${componentManifestPath}' must be a JSON object`);
      }
      const context = `component manifest '${componentManifestPath}'`;
      const capabilityId = requiredString(record, "capability_id", context);
      const capabilityVersion = requiredString(record, "capability_version", context);
      const executionMode = optionalString(record, "execution_mode") ?? "wasm";

      if (executionMode === "compatible") {
        const platforms = stringArray(record["platforms"], `${context} platforms`);
        if (platforms.length === 0) {
          throw loadFailure(`${context} declares execution_mode 'compatible' but no platforms`);
        }
        compatibleTargets.set(capabilityId, platforms);
        continue;
      }
      if (executionMode !== "wasm") {
        throw loadFailure(`${context} declares unsupported execution_mode '${executionMode}'`);
      }

      const wasmDigest = requiredString(record, "wasm_digest", context);
      if (!SHA256_DIGEST_PATTERN.test(wasmDigest)) {
        throw loadFailure(`${context} declares invalid wasm_digest metadata '${wasmDigest}'`);
      }
      if (wasmDigest.toLowerCase() !== component.digest.toLowerCase()) {
        throw loadFailure(
          `${context} wasm_digest does not match the app manifest's declared component digest`,
        );
      }
      const wasmBinaryPath = requiredString(record, "wasm_binary_path", context);
      const wasmPath = loader.resolve(componentManifestPath, wasmBinaryPath);
      let wasmBytes: Uint8Array;
      try {
        wasmBytes = await loader.loadBytes(wasmPath);
      } catch (error) {
        throw loadFailure(`failed to load WASM artifact '${wasmPath}': ${String(error)}`);
      }
      await verifyArtifactDigest(wasmBytes, wasmDigest, `component '${capabilityId}' artifact`);

      wasmTargets.set(capabilityId, {
        capabilityId,
        capabilityVersion,
        digest: wasmDigest,
        wasmBytes,
        serviceType: optionalString(record, "service_type"),
        emits: parseDeclaredEmits(record["emits"]),
      });
      wasmComponentEvidence.push({
        component_id: component.componentId,
        capability_id: capabilityId,
        wasm_digest: wasmDigest,
      });
    }

    const workflowTargets = new Map<string, WorkflowTarget>();
    for (const workflowRef of summary.workflows) {
      const workflowPath = loader.resolve(manifestPath, workflowRef.path);
      let workflowText: string;
      try {
        workflowText = await loader.loadText(workflowPath);
      } catch (error) {
        throw loadFailure(`failed to load workflow definition '${workflowPath}': ${String(error)}`);
      }
      let parsed: JsonValue;
      try {
        parsed = JSON.parse(workflowText) as JsonValue;
      } catch (error) {
        throw loadFailure(`workflow definition '${workflowPath}' is not valid JSON: ${String(error)}`);
      }
      const record = asRecord(parsed);
      if (record === null) {
        throw loadFailure(`workflow definition '${workflowPath}' must be a JSON object`);
      }
      const context = `workflow definition '${workflowPath}'`;
      const startNode = requiredString(record, "start_node", context);
      const outputProjection = stringArray(record["output_projection"], `${context} output_projection`);

      const nodesValue = record["nodes"];
      if (!Array.isArray(nodesValue)) {
        throw loadFailure(`${context} requires a 'nodes' array`);
      }
      const nodes = new Map<string, WorkflowNodeSpec>();
      for (const [index, entry] of nodesValue.entries()) {
        const node = asRecord(entry);
        if (node === null) {
          throw loadFailure(`${context} nodes[${index}] must be a JSON object`);
        }
        const nodeContext = `${context} nodes[${index}]`;
        const nodeId = requiredString(node, "node_id", nodeContext);
        const input = asRecord(node["input"]);
        const output = asRecord(node["output"]);
        nodes.set(nodeId, {
          nodeId,
          capabilityId: requiredString(node, "capability_id", nodeContext),
          capabilityVersion: requiredString(node, "capability_version", nodeContext),
          fromWorkflowInput: stringArray(
            input?.["from_workflow_input"],
            `${nodeContext}.input.from_workflow_input`,
          ),
          toWorkflowState: stringArray(
            output?.["to_workflow_state"],
            `${nodeContext}.output.to_workflow_state`,
          ),
          publishToStateAs: output !== null ? optionalString(output, "publish_to_state_as") : null,
        });
      }

      const edgesValue = record["edges"];
      const nextByFrom = new Map<string, string>();
      if (Array.isArray(edgesValue)) {
        for (const [index, entry] of edgesValue.entries()) {
          const edge = asRecord(entry);
          if (edge === null) {
            throw loadFailure(`${context} edges[${index}] must be a JSON object`);
          }
          const edgeContext = `${context} edges[${index}]`;
          const trigger = optionalString(edge, "trigger") ?? "direct";
          if (trigger !== "direct") {
            throw loadFailure(
              `${edgeContext} uses trigger '${trigger}'; this package version supports only ` +
                "'direct'-triggered linear pipelines",
            );
          }
          const from = requiredString(edge, "from", edgeContext);
          const to = requiredString(edge, "to", edgeContext);
          if (nextByFrom.has(from)) {
            throw loadFailure(
              `${edgeContext}: node '${from}' already has an outgoing direct edge; ` +
                "branching pipelines are not supported by this package version",
            );
          }
          nextByFrom.set(from, to);
        }
      }

      workflowTargets.set(workflowRef.workflowId, {
        version: workflowRef.workflowVersion,
        nodes,
        nextByFrom,
        startNode,
        outputProjection,
      });
    }

    const core = new EmbedderCore(
      config.workspaceId ?? "local-default",
      summary.appId,
      summary.appVersion,
      config.platform ?? "web",
      compatibleTargets,
    );
    return new BundleEmbedder(
      core,
      runtimeModule,
      runtimeDigest,
      wasmTargets,
      workflowTargets,
      wasmComponentEvidence,
      summary.stateMachine,
    );
  }

  submit(targetId: string, input: JsonValue): SubmitOutcome;
  submit(envelope: AppCommandEnvelope): SubmitOutcome;
  submit(
    targetIdOrEnvelope: string | AppCommandEnvelope,
    input?: JsonValue,
  ): SubmitOutcome {
    if (typeof targetIdOrEnvelope !== "string") {
      return this.submitAppCommand(targetIdOrEnvelope);
    }
    const targetId = targetIdOrEnvelope;
    if (input === undefined) {
      return this.core.rejectedSubmit(
        targetId,
        embedderError("invalid_app_command", "workflow/capability submit requires an input object"),
      );
    }
    if (this.core.stopped) {
      return this.core.rejectedSubmit(targetId, runtimeStoppedError());
    }
    if (this.workflowTargets.has(targetId)) {
      return this.submitWorkflow(targetId, input);
    }
    if (this.wasmTargets.has(targetId)) {
      return this.submitCapability(targetId, input);
    }
    if (this.core.compatibleTargets.has(targetId)) {
      return this.core.rejectedSubmit(
        targetId,
        embedderError(
          "compatible_lifecycle_required",
          `capability '${targetId}' is a compatible-mode capability; use compatible.start/stop/kill`,
        ),
      );
    }
    return this.core.rejectedSubmit(
      targetId,
      embedderError(
        "target_not_found",
        `'${targetId}' is neither a bundled workflow nor a bundled capability`,
      ),
    );
  }

  private submitAppCommand(envelope: AppCommandEnvelope): SubmitOutcome {
    if (this.core.stopped) {
      return this.core.rejectedSubmit("app_command", runtimeStoppedError());
    }
    if (envelope.kind !== "app_command") {
      return this.core.rejectedSubmit(
        "app_command",
        embedderError("invalid_app_command", "app command envelope requires kind 'app_command'"),
      );
    }
    if (typeof envelope.command !== "string" || envelope.command.trim() === "") {
      return this.core.rejectedSubmit(
        "app_command",
        embedderError("invalid_app_command", "app_command requires a non-empty command"),
      );
    }
    if (this.stateMachine === null) {
      return this.core.rejectedSubmit(
        "app_command",
        embedderError(
          "app_state_machine_unavailable",
          "application bundle does not declare a state_machine",
        ),
      );
    }
    let host: RuntimeWasmHost;
    try {
      host = this.ensureAppHost();
    } catch (error) {
      const message =
        error instanceof RuntimeWasmHostError
          ? error.message
          : `app orchestrator init failed: ${String(error)}`;
      return this.core.rejectedSubmit(
        "app_command",
        embedderError("app_state_machine_unavailable", message),
      );
    }

    const wire: Record<string, JsonValue> = {
      kind: "app_command",
      command: envelope.command,
      payload: envelope.payload ?? {},
    };
    if (envelope.sessionId !== undefined && envelope.sessionId !== null) {
      wire.session_id = envelope.sessionId;
    }

    let response: RuntimeWasmJson;
    try {
      response = host.submit(new TextEncoder().encode(JSON.stringify(wire)));
    } catch (error) {
      const message =
        error instanceof RuntimeWasmHostError
          ? error.message
          : `app_command submit failed: ${String(error)}`;
      return this.core.rejectedSubmit(
        "app_command",
        embedderError("invalid_app_command", message),
      );
    }

    const responseRecord =
      typeof response === "object" && response !== null && !Array.isArray(response)
        ? response
        : null;
    const sessionId =
      responseRecord && typeof responseRecord.session_id === "string"
        ? responseRecord.session_id
        : this.core.nextSessionId();
    const status = responseRecord?.status === "rejected" ? "rejected" : "accepted";
    if (status === "rejected") {
      const errorMessage =
        typeof responseRecord?.error === "string"
          ? responseRecord.error
          : "app_command rejected";
      return this.core.rejectedSubmit(
        "app_command",
        embedderError("invalid_app_command", errorMessage),
      );
    }

    let drained: RuntimeWasmJson[] = [];
    try {
      drained = host.drainEvents();
    } catch {
      drained = [];
    }
    this.emitAppEvents(drained, sessionId);
    this.registerAppDeadlines(host, responseRecord);

    return { sessionId, status: "accepted", error: null };
  }

  /**
   * Registers the host-side half of Spec 139's dual deadline. The timer is
   * deliberately outside runtime.wasm: browser clocks are host authority;
   * runtime.wasm only receives the correlated terminal envelope.
   */
  private registerAppDeadlines(
    host: RuntimeWasmHost,
    response: { readonly [key: string]: RuntimeWasmJson } | null,
  ): void {
    const deadlines = response?.pending_deadlines;
    if (!Array.isArray(deadlines)) return;
    for (const deadline of deadlines) {
      if (deadline === null || typeof deadline !== "object" || Array.isArray(deadline)) continue;
      const commandId = typeof deadline.command_id === "string" ? deadline.command_id : null;
      const sessionId = typeof deadline.session_id === "string" ? deadline.session_id : null;
      const delayMs = typeof deadline.deadline_ms === "number" ? deadline.deadline_ms : null;
      if (commandId === null || sessionId === null || delayMs === null || !Number.isFinite(delayMs)) {
        continue;
      }
      const timer = setTimeout(() => {
        this.appDeadlineTimers.delete(timer);
        if (this.core.stopped || this.appHost !== host) return;
        try {
          host.submit(new TextEncoder().encode(JSON.stringify({
            kind: "deadline_fired",
            command_id: commandId,
            session_id: sessionId,
          })));
          this.emitAppEvents(host.drainEvents(), sessionId);
        } catch {
          // A stopped/replaced runtime cannot accept a terminal; shutdown is
          // authoritative and no UI-owned recovery path is introduced.
        }
      }, Math.max(0, delayMs));
      this.appDeadlineTimers.add(timer);
    }
  }

  private emitAppEvents(events: readonly RuntimeWasmJson[], fallbackSessionId: string): void {
    for (const event of events) {
      if (typeof event !== "object" || event === null || Array.isArray(event)) continue;
      const rawType = typeof event.type === "string" ? event.type : "error";
      const eventSession = typeof event.session_id === "string" ? event.session_id : fallbackSessionId;
      this.core.emit(mapAppLifecycleEventType(rawType), eventSession, (event.data ?? {}) as JsonValue);
    }
  }

  private ensureAppHost(): RuntimeWasmHost {
    if (this.appHost !== null) {
      return this.appHost;
    }
    if (this.stateMachine === null) {
      throw new RuntimeWasmHostError("state_machine missing");
    }
    const nested = primaryInvokeWasmTarget(this.stateMachine, this.wasmTargets);
    if (nested === null) {
      throw new RuntimeWasmHostError(
        "state_machine invoke capability is not present in the bundle",
      );
    }
    const host = RuntimeWasmHost.fromModule(this.runtimeModule);
    host.init(
      {
        capabilityId: this.core.appId,
        capabilityVersion: this.core.appVersion,
        serviceType: mapServiceType(nested.serviceType),
        emits: nested.emits,
        hostPlacementTarget: "browser",
        permittedTargets: ["browser"],
        stateMachine: this.stateMachine as RuntimeWasmJson,
      },
      nested.wasmBytes,
    );
    this.appHost = host;
    return host;
  }

  private submitCapability(targetId: string, input: JsonValue): SubmitOutcome {

    const target = this.wasmTargets.get(targetId);
    if (target === undefined) {
      return this.core.rejectedSubmit(
        targetId,
        embedderError("target_not_found", `'${targetId}' is not a bundled capability`),
      );
    }
    const sessionId = this.core.nextSessionId();
    const requestId = this.core.nextRequestId();
    const executionId = `exec_${requestId}`;
    this.core.emit("capability_invoked", sessionId, {
      execution_id: executionId,
      capability_id: targetId,
      capability_version: target.capabilityVersion,
    });

    if (target.serviceType === "stateful") {
      const attestation = attestStatefulBrowserActivation(this.indexedDbDataStore);
      if (!attestation.ok) {
        this.core.recordTrace({
          executionId,
          targetId,
          outcome: "error",
          phases: [{ code: "error" }],
          selectedTarget: { targetId, targetVersion: target.capabilityVersion },
          placement: { target: "browser" },
          failureCode: attestation.error.code,
          stateMachineValid: null,
        });
        this.core.emit("error", sessionId, {
          execution_id: executionId,
          capability_id: targetId,
          status: "error",
          error: {
            code: attestation.error.code,
            message: "Stateful Browser activation requires an open Spec 085 IndexedDB DataStore",
            details: {
              governing_spec: attestation.error.governing_spec,
              outcome: attestation.error.outcome,
              reason: attestation.error.reason,
            },
          },
        });
        return { sessionId, status: "accepted", error: null };
      }
    }

    const result = executeWasmModule(this.runtimeModule, target, input, (event) => {
      this.core.emit("capability_event", sessionId, {
        execution_id: executionId,
        capability_id: targetId,
        event_id: event.event_id,
        version: event.version,
        payload: event.payload,
      });
    });
    this.core.recordTrace({
      executionId,
      targetId,
      outcome: result.ok ? "completed" : "error",
      phases: [{ code: result.ok ? "completed" : "error" }],
      selectedTarget: { targetId, targetVersion: target.capabilityVersion },
      placement: { target: "browser" },
      failureCode: result.ok ? null : result.code,
      stateMachineValid: null,
    });
    if (result.ok) {
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

  private submitWorkflow(targetId: string, input: JsonValue): SubmitOutcome {
    const workflow = this.workflowTargets.get(targetId);
    if (workflow === undefined) {
      return this.core.rejectedSubmit(
        targetId,
        embedderError("target_not_found", `'${targetId}' is not a bundled workflow`),
      );
    }
    const sessionId = this.core.nextSessionId();
    const requestId = this.core.nextRequestId();

    const state = initialWorkflowState(input);
    const steps: WorkflowStepRecord[] = [];
    let failure: { code: string; message: string } | null = null;
    let stepIndex = 0;
    let currentNodeId: string | undefined = workflow.startNode;

    while (currentNodeId !== undefined) {
      const node = workflow.nodes.get(currentNodeId);
      if (node === undefined) {
        failure = {
          code: "execution_failed",
          message: `workflow node '${currentNodeId}' could not be resolved during traversal`,
        };
        break;
      }
      const target = this.wasmTargets.get(node.capabilityId);
      if (target === undefined) {
        steps.push({
          stepIndex,
          nodeId: node.nodeId,
          capabilityId: node.capabilityId,
          capabilityVersion: node.capabilityVersion,
          status: "failed",
        });
        failure = {
          code: "capability_not_found",
          message: `capability '${node.capabilityId}' is not a bundled WASM capability`,
        };
        break;
      }
      const nodeInput = buildNodeInput(state, node.fromWorkflowInput);
      if (target.serviceType === "stateful") {
        const attestation = attestStatefulBrowserActivation(this.indexedDbDataStore);
        if (!attestation.ok) {
          steps.push({
            stepIndex,
            nodeId: node.nodeId,
            capabilityId: node.capabilityId,
            capabilityVersion: node.capabilityVersion,
            status: "failed",
          });
          failure = {
            code: attestation.error.code,
            message:
              "Stateful Browser activation requires an open Spec 085 IndexedDB DataStore",
          };
          break;
        }
      }
      const result = executeWasmModule(this.runtimeModule, target, nodeInput, (event) => {
        this.core.emit("capability_event", sessionId, {
          execution_id: `exec_${requestId}`,
          capability_id: node.capabilityId,
          node_id: node.nodeId,
          event_id: event.event_id,
          version: event.version,
          payload: event.payload,
        });
      });
      if (!result.ok) {
        steps.push({
          stepIndex,
          nodeId: node.nodeId,
          capabilityId: node.capabilityId,
          capabilityVersion: node.capabilityVersion,
          status: "failed",
        });
        failure = { code: result.code, message: result.message };
        break;
      }
      steps.push({
        stepIndex,
        nodeId: node.nodeId,
        capabilityId: node.capabilityId,
        capabilityVersion: node.capabilityVersion,
        status: "completed",
      });
      applyNodeOutput(state, node, result.output as JsonValue);
      stepIndex += 1;
      currentNodeId = workflow.nextByFrom.get(node.nodeId);
    }

    this.core.recordTrace({
      executionId: `workflow-${requestId}`,
      targetId,
      outcome: failure === null ? "completed" : "error",
      phases: steps.map((step) => ({ code: `workflow_${step.status}` })),
      selectedTarget: { targetId, targetVersion: workflow.version },
      placement: { target: "browser" },
      failureCode: failure?.code ?? null,
      stateMachineValid: null,
    });

    for (const step of steps) {
      this.core.emit("capability_invoked", sessionId, {
        request_id: requestId,
        workflow_id: targetId,
        workflow_version: workflow.version,
        step_index: step.stepIndex,
        node_id: step.nodeId,
        capability_id: step.capabilityId,
        capability_version: step.capabilityVersion,
        status: step.status,
      });
    }
    if (failure === null) {
      this.core.emit("capability_result", sessionId, {
        request_id: requestId,
        workflow_id: targetId,
        workflow_version: workflow.version,
        status: "completed",
        output: projectWorkflowOutput(state, workflow.outputProjection),
      });
    } else {
      this.core.emit("error", sessionId, {
        request_id: requestId,
        workflow_id: targetId,
        workflow_version: workflow.version,
        status: "error",
        error: { code: failure.code, message: failure.message, details: {} },
      });
    }
    return { sessionId, status: "accepted", error: null };
  }

  subscribe(callback: EventCallback): void {
    this.core.subscribe(callback);
  }

  embeddedTraceApiVersion(): string {
    return EMBEDDED_TRACE_API_VERSION;
  }

  traceList(
    requestedVersion: string,
    pageSize: number,
    cursor: string | null = null,
  ): EmbeddedTracePage | EmbeddedTraceApiError {
    return this.core.traceList(requestedVersion, pageSize, cursor);
  }

  traceGet(
    requestedVersion: string,
    traceId: string,
  ): EmbeddedTraceDetail | EmbeddedTraceApiError {
    return this.core.traceGet(requestedVersion, traceId);
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
    for (const timer of this.appDeadlineTimers) clearTimeout(timer);
    this.appDeadlineTimers.clear();
    if (this.appHost !== null) {
      try {
        this.appHost.shutdown();
      } catch {
        // Best-effort; core.shutdown remains authoritative for the public surface.
      }
      this.appHost = null;
    }

    return this.core.shutdown();
  }

  releaseEvidence(): JsonValue {
    const evidence = this.core.evidence("browser-webassembly", [
      ...this.wasmComponentEvidence,
    ]) as { readonly [key: string]: JsonValue };
    return {
      ...evidence,
      runtime: {
        implementation: "browser-webassembly",
        runtime_wasm_digest: this.runtimeDigest,
      },
    };
  }
}

/**
 * Drives one capability through a fresh `runtime.wasm` instance:
 * instantiate → init → submit → drainEvents → map domain events.
 */
function executeWasmModule(
  runtimeModule: WebAssembly.Module,
  target: WasmTarget,
  input: JsonValue,
  onCapabilityEvent: (event: {
    event_id: string;
    version: string;
    payload: JsonValue;
  }) => void,
): WasmExecutionResult {
  let host: RuntimeWasmHost;
  try {
    host = RuntimeWasmHost.fromModule(runtimeModule);
  } catch (error) {
    const message =
      error instanceof RuntimeWasmHostError
        ? error.message
        : `runtime.wasm instantiate failed: ${String(error)}`;
    return { ok: false, output: null, ...classifyRuntimeFailure(message) };
  }

  try {
    host.init(
      {
        capabilityId: target.capabilityId,
        capabilityVersion: target.capabilityVersion,
        serviceType: mapServiceType(target.serviceType),
        emits: target.emits,
        hostPlacementTarget: "browser",
        permittedTargets: ["browser"],
      },
      target.wasmBytes,
    );
  } catch (error) {
    const message =
      error instanceof RuntimeWasmHostError
        ? error.message
        : `runtime.wasm init failed: ${String(error)}`;
    return { ok: false, output: null, ...classifyRuntimeFailure(message) };
  }

  try {
    host.submit(new TextEncoder().encode(JSON.stringify(input)));
  } catch (error) {
    const message =
      error instanceof RuntimeWasmHostError
        ? error.message
        : `runtime.wasm submit failed: ${String(error)}`;
    return { ok: false, output: null, ...classifyRuntimeFailure(message) };
  }

  let drained: RuntimeWasmJson[];
  try {
    drained = host.drainEvents();
  } catch (error) {
    const message =
      error instanceof RuntimeWasmHostError
        ? error.message
        : `runtime.wasm drainEvents failed: ${String(error)}`;
    return { ok: false, output: null, ...classifyRuntimeFailure(message) };
  }

  try {
    host.shutdown();
  } catch {
    // Best-effort cleanup for a one-shot instance.
  }

  const mapped = mapRuntimeWasmEvents(drained);
  let result: WasmExecutionResult | null = null;
  for (const event of mapped) {
    if (event.type === "capability_event") {
      const data =
        event.data !== null && typeof event.data === "object" && !Array.isArray(event.data)
          ? (event.data as { readonly [key: string]: RuntimeWasmJson })
          : {};
      const eventId = typeof data["event_type"] === "string" ? data["event_type"] : null;
      const version = typeof data["version"] === "string" ? data["version"] : "0.0.0";
      if (eventId !== null) {
        onCapabilityEvent({
          event_id: eventId,
          version,
          payload: asJsonValue(data["payload"] ?? {}),
        });
      }
      continue;
    }
    if (event.type !== "capability_result") {
      continue;
    }
    const data =
      event.data !== null && typeof event.data === "object" && !Array.isArray(event.data)
        ? (event.data as { readonly [key: string]: RuntimeWasmJson })
        : {};
    const status = typeof data["status"] === "string" ? data["status"] : null;
    if (status === "completed") {
      result = {
        ok: true,
        output: asJsonValue(data["output"] ?? null),
        code: "",
        message: "",
      };
      continue;
    }
    const errorText =
      typeof data["error"] === "string"
        ? data["error"]
        : `capability_result status '${status ?? "unknown"}'`;
    result = { ok: false, output: null, ...classifyRuntimeFailure(errorText) };
  }

  if (result === null) {
    return {
      ok: false,
      output: null,
      code: "execution_failed",
      message: "runtime.wasm produced no capability_result event",
    };
  }
  return result;
}

/**
 * Shared wire types for the `embedder-api/1.0.0` boundary (spec 057). These
 * are wire-identical in field naming to the Rust `traverse-embedder` crate
 * so every platform observes the same operations, event envelope, and error
 * codes (spec 057 FR-003).
 */

/** Implemented embedder API version (spec 057 IDL `$id` suffix). */
export const EMBEDDER_API_VERSION = "1.0.0";

/** Conformance suite revision this package certifies against (spec 057). */
export const EMBEDDER_CONFORMANCE_VERSION = "1.0.0";

/** Application bundle manifest `schema_version` values this package accepts. */
export const SUPPORTED_BUNDLE_SCHEMA_VERSIONS: readonly string[] = ["1.0.0"];

export const EVENT_SCHEMA_VERSION = "1.0.0";
export const PACKAGE_NAME = "traverse-embedder-web";
export const PACKAGE_VERSION = "0.7.0";

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
 * required by spec 068 FR-006; `BundleEmbedder` is the production
 * runtime-WASM implementation. Both implement this identical boundary.
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

export function embedderError(code: EmbedderErrorCode, message: string): EmbedderError {
  return { code, message };
}

export function errorValue(error: EmbedderError): JsonValue {
  return { code: error.code, message: error.message };
}

export function paddedId(prefix: string, counter: number): string {
  return `${prefix}-${String(counter).padStart(8, "0")}`;
}

export function runtimeStoppedError(): EmbedderError {
  return embedderError(
    "runtime_stopped",
    "the embedded runtime was shut down and accepts no further operations",
  );
}

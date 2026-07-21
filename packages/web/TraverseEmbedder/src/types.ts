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

/** Implemented additive public Trace API companion version (spec 517). */
export const EMBEDDED_TRACE_API_VERSION = "1.0.0";

/** Maximum public trace records retained by one embedded session. */
export const EMBEDDED_TRACE_RETENTION_LIMIT = 100;

/** Largest `trace.list` page returned by this package. */
export const EMBEDDED_TRACE_MAX_PAGE_SIZE = 100;

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

/** Stable public Trace API failures (spec 517 FR-010). */
export type EmbeddedTraceApiErrorCode =
  | "invalid_cursor"
  | "trace_not_found"
  | "trace_api_unavailable"
  | "incompatible_version";

/** A safe public Trace API failure. It never contains runtime error details. */
export interface EmbeddedTraceApiError {
  readonly code: EmbeddedTraceApiErrorCode;
  readonly message: string;
}

/** Safe terminal outcome exposed through the public Trace API. */
export type EmbeddedTraceOutcome = "completed" | "error";

/** A safe phase classification with no event payload or telemetry attributes. */
export interface EmbeddedTracePhase {
  readonly code: string;
}

/** Safe selected-target evidence from a completed execution. */
export interface EmbeddedTraceSelectedTarget {
  readonly targetId: string;
  readonly targetVersion: string | null;
}

/** Safe placement evidence from a completed execution. */
export interface EmbeddedTracePlacement {
  readonly target: string;
}

/** Safe list-oriented public record for one completed local execution. */
export interface EmbeddedTraceSummary {
  readonly traceId: string;
  readonly executionId: string;
  readonly targetId: string;
  readonly completedAt: string;
  readonly completionSequence: number;
  readonly outcome: EmbeddedTraceOutcome;
}

/** Safe public diagnostic detail for one retained local trace. */
export interface EmbeddedTraceDetail {
  readonly summary: EmbeddedTraceSummary;
  readonly phases: readonly EmbeddedTracePhase[];
  readonly selectedTarget: EmbeddedTraceSelectedTarget | null;
  readonly placement: EmbeddedTracePlacement | null;
  readonly failureCode: string | null;
  readonly stateMachineValid: boolean | null;
}

/** Cursor-paged, bounded public Trace API response. */
export interface EmbeddedTracePage {
  readonly summaries: readonly EmbeddedTraceSummary[];
  readonly nextCursor: string | null;
  readonly retentionLimit: number;
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

/**
 * The additive `embedded-trace-api/1.0.0` companion surface (spec 517).
 * It is intentionally separate from `TraverseEmbedderApi` so baseline hosts
 * and external implementers retain their existing `embedder-api/1.0.0`
 * compatibility.
 */
export interface EmbeddedTraceApi {
  /** Advertised companion API version. */
  embeddedTraceApiVersion(): string;
  /** `trace.list`: safe newest-first summaries for this local session. */
  traceList(
    requestedVersion: string,
    pageSize: number,
    cursor?: string | null,
  ): EmbeddedTracePage | EmbeddedTraceApiError;
  /** `trace.get`: one safe retained detail by opaque public trace ID. */
  traceGet(
    requestedVersion: string,
    traceId: string,
  ): EmbeddedTraceDetail | EmbeddedTraceApiError;
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

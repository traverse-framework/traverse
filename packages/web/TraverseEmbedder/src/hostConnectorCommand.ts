/**
 * Shared Spec 137 host-connector command/event contract for browser and macOS.
 * Host adapters stay out of this module; they remain platform implementations.
 */

export const HOST_CONNECTOR_COMMAND_SCHEMA_VERSION = "1.0.0" as const;
export const HOST_CONNECTOR_COMMAND_KIND = "host_connector_command" as const;
export const HOST_CONNECTOR_RESULT_KIND = "host_connector_result" as const;
export const HOST_CONNECTOR_EVENT_KIND = "host_connector_event" as const;
export const HOST_CONNECTOR_GOVERNING_SPEC =
  "137-host-connector-command-dispatch" as const;
export const AUDIO_INPUT_CONNECTOR = "traverse.audio-input" as const;
export const AUDIO_CAPTURE_OPERATION = "audio.capture" as const;
export const AUDIO_PERMISSION_REQUEST_OPERATION =
  "audio.permission.request" as const;
export const MODEL_RUNTIME_CONNECTOR = "traverse.model-runtime" as const;
export const MODEL_EXECUTE_OPERATION = "model.execute" as const;

export type HostConnectorTargetFamily = "browser" | "macos" | "local";

export type HostConnectorErrorCode =
  | "unknown_command"
  | "unbound"
  | "incompatible"
  | "unconfigured"
  | "target_incompatible"
  | "input_limit_exceeded"
  | "cancelled"
  | "idempotency_conflict"
  | "policy_denied"
  | "unavailable"
  | "invalid_input"
  | "model_unavailable"
  | "model_incompatible"
  | "resource_exhausted"
  | "timeout"
  | "execution_failed";

/** Spec 138 exact model pin carried on `model.execute`. */
export interface ModelRef {
  readonly model_id: string;
  readonly version: string;
  readonly digest: string;
}

/** Spec 138 `model.execute` request payload (tensors via host-staged refs). */
export interface ModelExecutePayload {
  readonly model_ref: ModelRef;
  readonly input_ref: string;
  readonly policy_ref: string;
  readonly data_classification: string;
  readonly input_schema_ref: string;
  readonly input_schema_version: string;
  readonly max_output_bytes: number;
  readonly max_memory_bytes?: number;
  readonly max_fuel?: number;
  readonly timeout_ms?: number;
  readonly feature_metadata?: Record<string, unknown>;
}

export const MODEL_RUNTIME_GOVERNING_SPEC =
  "138-governed-exact-model-execution" as const;
export const PLACEMENT_WASM_CPU = "wasm-cpu" as const;

/**
 * Normalize a Spec 138 model.execute success/failure for cross-target compare.
 * Strips host-private fields; keeps status-bearing public codes and identity.
 */
export function normalizeModelExecuteEvidence(input: {
  readonly status?: string;
  readonly error_code?: HostConnectorErrorCode;
  readonly model_ref?: ModelRef;
  readonly placement?: string;
  readonly output_ref?: string;
  readonly artifact_ref?: string;
}): Record<string, unknown> {
  return {
    governing_spec: MODEL_RUNTIME_GOVERNING_SPEC,
    status: input.status ?? (input.error_code ? "failed" : "ok"),
    error_code: input.error_code ?? null,
    model_ref: input.model_ref ?? null,
    placement: input.placement ?? PLACEMENT_WASM_CPU,
    has_output_ref: Boolean(input.output_ref ?? input.artifact_ref),
  };
}

export function modelExecuteCommand(
  commandId: string,
  correlationId: string,
  idempotencyKey: string,
  targetFamily: HostConnectorTargetFamily,
  payload: ModelExecutePayload,
): HostConnectorAppCommand {
  return {
    kind: HOST_CONNECTOR_COMMAND_KIND,
    schema_version: HOST_CONNECTOR_COMMAND_SCHEMA_VERSION,
    command: "run_local_model",
    command_id: commandId,
    correlation_id: correlationId,
    idempotency_key: idempotencyKey,
    target_family: targetFamily,
    cancel_requested: false,
    payload: { ...payload },
  };
}

export type HostConnectorEventName =
  | "accepted"
  | "started"
  | "completed"
  | "cancelled"
  | "failed";

export interface HostConnectorAppCommand {
  readonly kind: typeof HOST_CONNECTOR_COMMAND_KIND;
  readonly schema_version: typeof HOST_CONNECTOR_COMMAND_SCHEMA_VERSION;
  readonly command: string;
  readonly command_id: string;
  readonly correlation_id: string;
  readonly idempotency_key: string;
  readonly target_family: HostConnectorTargetFamily;
  readonly cancel_requested?: boolean;
  readonly payload: Record<string, unknown>;
}

export interface HostConnectorEvent {
  readonly kind: typeof HOST_CONNECTOR_EVENT_KIND;
  readonly schema_version: typeof HOST_CONNECTOR_COMMAND_SCHEMA_VERSION;
  readonly event: HostConnectorEventName;
  readonly command_id: string;
  readonly correlation_id: string;
  readonly connector_id?: string;
  readonly operation?: string;
  readonly binding_id?: string;
  readonly target_family: HostConnectorTargetFamily;
  readonly artifact_ref?: string;
  readonly error_code?: HostConnectorErrorCode;
}

export interface HostConnectorError {
  readonly code: HostConnectorErrorCode;
  readonly message: string;
}

export function audioCaptureCommand(
  commandId: string,
  correlationId: string,
  idempotencyKey: string,
  targetFamily: HostConnectorTargetFamily,
  payload: { readonly max_duration_ms: number; readonly max_bytes: number },
): HostConnectorAppCommand {
  return {
    kind: HOST_CONNECTOR_COMMAND_KIND,
    schema_version: HOST_CONNECTOR_COMMAND_SCHEMA_VERSION,
    command: "capture_audio",
    command_id: commandId,
    correlation_id: correlationId,
    idempotency_key: idempotencyKey,
    target_family: targetFamily,
    cancel_requested: false,
    payload: { ...payload },
  };
}

export function audioPermissionCommand(
  commandId: string,
  correlationId: string,
  idempotencyKey: string,
  targetFamily: HostConnectorTargetFamily,
): HostConnectorAppCommand {
  return {
    kind: HOST_CONNECTOR_COMMAND_KIND,
    schema_version: HOST_CONNECTOR_COMMAND_SCHEMA_VERSION,
    command: "request_permission",
    command_id: commandId,
    correlation_id: correlationId,
    idempotency_key: idempotencyKey,
    target_family: targetFamily,
    cancel_requested: false,
    payload: {},
  };
}

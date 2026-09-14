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
  | "unavailable";

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

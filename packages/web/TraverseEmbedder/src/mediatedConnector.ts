/**
 * Spec 104 mediated connectors a host answers asynchronously (Spec 1719,
 * Decision 122), and the first-party `traverse.model-runtime` binding.
 */
import { ExactModelError } from "./exactModel.js";
import type { ExactModelBrowserHost } from "./exactModel.js";
import { MODEL_EXECUTE_OPERATION, MODEL_RUNTIME_CONNECTOR } from "./hostConnectorCommand.js";
import type { JsonValue } from "./types.js";

/** Connector contract version the shipped model binding serves. */
export const MODEL_RUNTIME_CONNECTOR_VERSION = "2.3.0";

const HOST_ABI_VERSION = "1.0.0";

/** A guest's `connector_invoke` request. */
export interface MediatedConnectorRequest {
  readonly abi_version: string;
  readonly connector_id: string;
  readonly operation: string;
  readonly payload: JsonValue;
}

/** The answer returned to the guest, after the Spec 104 checks. */
export interface MediatedConnectorResponse {
  readonly abi_version: string;
  readonly result_class: string;
  readonly payload: JsonValue;
}

/** A host-owned connector. A guest never sees the implementation. */
export interface MediatedConnector {
  invoke(
    request: MediatedConnectorRequest,
    context: { readonly signal?: AbortSignal },
  ): Promise<MediatedConnectorResponse>;
}

export interface ExactModelMediatedConnectorOptions {
  /** Data classifications the host allows guests to send to a model. */
  readonly allowedClassifications: readonly string[];
}

function failure(resultClass: string, error: { [key: string]: JsonValue }): MediatedConnectorResponse {
  return {
    abi_version: HOST_ABI_VERSION,
    result_class: resultClass,
    payload: { status: "error", error },
  };
}

/** Wraps a configured `ExactModelBrowserHost`, with the native binding's responses. */
export function exactModelMediatedConnector(
  host: ExactModelBrowserHost,
  options: ExactModelMediatedConnectorOptions,
): MediatedConnector {
  return {
    async invoke(request, { signal }) {
      if (request.connector_id !== MODEL_RUNTIME_CONNECTOR || request.operation !== MODEL_EXECUTE_OPERATION) {
        return failure("incompatible", {
          code: "incompatible",
          message: "exactModelMediatedConnector only serves traverse.model-runtime model.execute",
        });
      }
      const payload = request.payload as unknown as Parameters<ExactModelBrowserHost["execute"]>[0];
      try {
        const execution = await host.execute({
          ...payload,
          allowed_classifications: options.allowedClassifications,
          ...(signal !== undefined ? { signal } : {}),
        });
        return {
          abi_version: HOST_ABI_VERSION,
          result_class: "success",
          payload: {
            status: "ok",
            output_ref: execution.output_ref,
            output_schema_ref: execution.output_schema_ref,
            model_ref: (request.payload as { model_ref?: JsonValue }).model_ref ?? null,
            placement: execution.placement,
            result_class: "success",
            model_evidence: execution.model_evidence as unknown as JsonValue,
          },
        };
      } catch (error) {
        if (!(error instanceof ExactModelError)) {
          throw error;
        }
        return failure(error.reason ?? error.code, {
          code: error.code,
          message: error.message,
          ...(error.reason !== undefined ? { reason: error.reason } : {}),
          ...(error.detail !== undefined ? { detail: error.detail as unknown as JsonValue } : {}),
        });
      }
    },
  };
}

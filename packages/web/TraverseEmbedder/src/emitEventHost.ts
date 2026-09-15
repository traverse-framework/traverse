/**
 * Browser host implementation of `traverse_host::emit_event` (spec
 * `098-capability-event-host-abi`), mirroring
 * `crates/traverse-runtime/src/executor/wasm.rs` `handle_emit_event`.
 *
 * INTERIM (issue #1404 / Decision 86): this TypeScript reimplementation is a
 * stopgap until Spec `1402-runtime-wasm-orchestrator-convergence` replaces
 * `BundleEmbedder`'s hand-rolled executor with a real `runtime.wasm`
 * orchestrator that reuses the Rust `handle_emit_event` path. See #1402.
 */

import type { JsonValue } from "./types.js";
import type { WasiMemoryRef } from "./wasi.js";

/** Spec 098 FR-008: max guest payload size before any memory read. */
export const MAX_EVENT_EMIT_PAYLOAD_BYTES = 64 * 1024;

export const EMIT_EVENT_OK = 0;
export const EMIT_EVENT_ERR_INVALID_PAYLOAD = -1;
export const EMIT_EVENT_ERR_UNDECLARED_EVENT = -2;
export const EMIT_EVENT_ERR_NOT_SUBSCRIBABLE = -3;

export interface DeclaredEmit {
  readonly event_id: string;
  readonly version: string;
}

export interface AcceptedCapabilityEvent {
  readonly event_id: string;
  readonly version: string;
  readonly payload: JsonValue;
}

export interface EmitEventHostContext {
  readonly capabilityId: string;
  readonly serviceType: string | null;
  readonly emits: readonly DeclaredEmit[];
  readonly onAccepted: (event: AcceptedCapabilityEvent) => void;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * Parse contract/manifest `emits` entries into `{ event_id, version }` pairs.
 * Unknown shapes are skipped (closed for undeclared checks).
 */
export function parseDeclaredEmits(value: unknown): DeclaredEmit[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const declared: DeclaredEmit[] = [];
  for (const entry of value) {
    const record = asRecord(entry);
    if (record === null) {
      continue;
    }
    const eventId = record["event_id"];
    const version = record["version"];
    if (typeof eventId === "string" && typeof version === "string") {
      declared.push({ event_id: eventId, version });
    }
  }
  return declared;
}

/**
 * Builds the `traverse_host.emit_event` import for one capability execution.
 * Never throws into the guest: every failure returns a negative status code.
 */
export function createEmitEventHostImport(
  context: EmitEventHostContext,
  memoryRef: WasiMemoryRef,
): (ptr: number, len: number) => number {
  // INTERIM pending #1402 — see module header.
  return (ptr: number, len: number): number => {
    if (context.serviceType !== "subscribable") {
      return EMIT_EVENT_ERR_NOT_SUBSCRIBABLE;
    }
    if (!Number.isInteger(ptr) || !Number.isInteger(len) || ptr < 0 || len < 0) {
      return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }
    if (len > MAX_EVENT_EMIT_PAYLOAD_BYTES) {
      return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }
    const memory = memoryRef.memory;
    if (memory === null) {
      return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }
    const bytes = new Uint8Array(memory.buffer);
    if (ptr + len > bytes.byteLength) {
      return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }
    let parsed: unknown;
    try {
      const text = new TextDecoder().decode(bytes.subarray(ptr, ptr + len));
      parsed = JSON.parse(text) as unknown;
    } catch {
      return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }
    const record = asRecord(parsed);
    if (record === null) {
      return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }
    const eventId = record["event_id"];
    const version = record["version"];
    if (typeof eventId !== "string" || typeof version !== "string") {
      return EMIT_EVENT_ERR_INVALID_PAYLOAD;
    }
    const declared = context.emits.some(
      (entry) => entry.event_id === eventId && entry.version === version,
    );
    if (!declared) {
      return EMIT_EVENT_ERR_UNDECLARED_EVENT;
    }
    const payloadValue = record["payload"];
    const payload: JsonValue =
      payloadValue === undefined
        ? {}
        : (payloadValue as JsonValue);
    context.onAccepted({ event_id: eventId, version, payload });
    return EMIT_EVENT_OK;
  };
}

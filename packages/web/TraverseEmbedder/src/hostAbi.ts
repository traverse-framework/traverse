/**
 * Traverse Host ABI import catalog (mirrors
 * `crates/traverse-runtime/src/executor/host_abi_v1.json`).
 *
 * Per-capability load-time whitelist enforcement lived in the interim
 * browser executor and was retired with spec `1402` FR-007: nested
 * `runtime.wasm` (wasmi) is now the sole linker for capability guests.
 * This module keeps the catalog as documentation / shared reference.
 */

export interface HostAbiImport {
  readonly module: string;
  readonly name: string;
}

/** Traverse Host ABI version nested `runtime.wasm` links against. */
export const SUPPORTED_HOST_ABI_VERSION = "1.0.0";

export const HOST_ABI_V1_WHITELIST: readonly HostAbiImport[] = [
  { module: "wasi_snapshot_preview1", name: "fd_read" },
  { module: "wasi_snapshot_preview1", name: "fd_write" },
  { module: "wasi_snapshot_preview1", name: "proc_exit" },
  { module: "traverse_host", name: "capability_id" },
  { module: "traverse_host", name: "capability_version" },
  { module: "traverse_host", name: "runtime_config" },
  { module: "traverse_host", name: "trace_context" },
  { module: "traverse_host", name: "execution_id" },
  { module: "traverse_host", name: "emit_event" },
  { module: "traverse_host", name: "connector_invoke" },
];

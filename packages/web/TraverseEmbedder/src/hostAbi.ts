/**
 * Traverse Host ABI import whitelist (mirrors
 * `crates/traverse-runtime/src/executor/host_abi_v1.json`). A bundled WASM
 * capability module may declare only these imports; every other import is
 * rejected deterministically before the module is instantiated — the
 * browser executor never links an unauthorized host function (deny-by-
 * default, matching the native `WasmExecutor`'s no-filesystem,
 * no-network, no-env-vars posture).
 */

export interface HostAbiImport {
  readonly module: string;
  readonly name: string;
}

/** Traverse Host ABI version this package validates modules against. */
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
];

/**
 * Returns the first function import outside the host ABI whitelist, or
 * `null` when every function import is authorized.
 */
export function findUnauthorizedImport(
  module: WebAssembly.Module,
): HostAbiImport | null {
  for (const imported of WebAssembly.Module.imports(module)) {
    if (imported.kind !== "function") {
      continue;
    }
    const allowed = HOST_ABI_V1_WHITELIST.some(
      (entry) => entry.module === imported.module && entry.name === imported.name,
    );
    if (!allowed) {
      return { module: imported.module, name: imported.name };
    }
  }
  return null;
}

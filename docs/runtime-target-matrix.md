# Runtime Target Matrix

Traverse separates runtime orchestration from native execution adapters so the
core runtime can be embedded in browser and edge-WASM environments.

| Target | Cargo flags | Supported surface |
| --- | --- | --- |
| Native host (`x86_64`/`aarch64` Linux, macOS, Windows) | default features | Full runtime orchestration, `NativeExecutor`, `ThreadPoolExecutor`, and Wasmtime-backed `WasmExecutor`. |
| Browser/edge core (`wasm32-unknown-unknown`) | `--no-default-features` | Core contracts, registry resolution, routing types, traces, events, security helpers, and host-provided executors. Native Wasmtime and Rayon adapters are excluded. |
| WASI guest capability (`wasm32-wasip1`) | capability crate target | Capability binaries authored for `WasmExecutor`; this is not the host runtime target. |

Default features preserve the native behavior expected by existing consumers.
Consumers embedding the runtime in a browser or edge-WASM guest should depend on
`traverse-runtime` with `default-features = false` and provide their own
executor implementation behind the existing `CapabilityExecutor` trait.

CI enforces the browser/edge core build with:

```bash
cargo check -p traverse-runtime --target wasm32-unknown-unknown --no-default-features
```

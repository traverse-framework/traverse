# Platform status — check before you promise a target

Traverse is pre-1.0 (runtime/crates v0.10.2). "Runs everywhere" is the design
goal, not the current state. **Shipped** means Spec 068 / Spec 136: a public
package on the language-default registry plus a no-sidecar reference path.
**Certified / Preview** is Spec 529 and is a later matrix; do not use 529 as a
Shipped gate. Cloud placement is a constitution v0.1 non-goal.

| Target | Status | Evidence |
|---|---|---|
| **Native** | Shipped | `traverse-runtime`: `NativeExecutor`, `ThreadPoolExecutor`, Wasmtime `WasmExecutor`. |
| **Browser** | Shipped | `traverse-embedder-web` on npm (currently **0.10.2**; lockstep with the runtime line). |
| **Rust app embedding** | Shipped | `traverse-embedder` **0.10.2** on crates.io; CI `embedder_conformance/rust_package.sh`. |
| **Swift / iOS** | Shipped | `WasmiHostBridgeClient` + `TraverseSwiftHost` XCFramework; iOS/macOS shells in `reference-apps`. Spec 074 Approved; ADR-0070 selects wasmi **2.0.0**. The SPM zip is still the **v0.8.2 / wasmi 1.1.0** asset until `#1370` records new device fixtures and Spec 136 publishes it on the same `v*` as Maven/NuGet. Not Blocked. |
| **Kotlin / Android** | In progress | `ChicoryRuntimeBridge` 1.7.5 + Compose shells exist (`TRAVERSE_REPO` path). **No Maven Central artifact.** `#1371`. |
| **.NET / WinUI** | In progress | `WasmtimeRuntimeBridge` 44.0.0 + WinUI shells exist (vendor path). **No nuget.org package.** `#1372`. |
| **Edge** | Planned / pre-spec | `ExecutionTarget::Edge` is a contract enum only. No Workers host. Spec 038 out-of-scopes edge adapters. Do not promise Cloudflare. |
| **Cloud placement** | Out of v0.1 | Constitution non-goals: distributed orchestration, edge/cloud placement optimization, multi-cloud runtime execution. Do not plan it here. |

## What this means for a capability you just built

- `permitted_targets: ["local"]` is honest for almost every new capability today. Add `browser` only after you have tested the browser embedder path.
- Omitting `permitted_targets` (schema default = all targets) is a governance claim, not proof the capability runs on edge or cloud.
- iOS: yes, via the Swift package and current XCFramework. Tell people the zip is still v0.8.2 until `#1370` lands; do not say the engine is blocked.
- Android / Windows: the embedder works from this repo or App-Refs path/vendor deps. A stranger cannot `implementation` / `PackageReference` a public coordinate yet.
- MCP (`traverse-mcp`) is a separate axis from placement.

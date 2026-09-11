# Platform status — check before you promise a target

Traverse is pre-1.0 (v0.8.1). "Runs everywhere" is the design goal, not the current state. Before telling a user their capability will work on some target, check it against this table — it's condensed from `/platforms.html` on the docs site plus direct code verification (not just marketing copy), and it changes release to release, so treat the docs site as the source of truth if this file and the live site ever disagree.

| Target | Status | Evidence |
|---|---|---|
| **Native** | Shipped | `traverse-runtime` crate, Wasmtime-backed `WasmExecutor` — this is the real executor that actually runs a capability's WASM. |
| **Browser** | Shipped | Web/TypeScript embedder SDK, `traverse-embedder-web` package, `BundleEmbedder` class, currently v0.8.0. |
| **Rust app embedding** | Shipped | `traverse-embedder` crate. `BundleEmbedder::init` / `.submit` / `.subscribe` / `.shutdown` genuinely call through to the real `ArtifactRouter` executor — this is not a stub. |
| **Swift / iOS** | Blocked | No WASM engine certified as compliant. WasmKit, WAMR, Wasmtime, and Wasmer were all screened and rejected — this isn't "not started yet," it's a currently unresolved dependency problem upstream of Traverse itself. |
| **Kotlin / Android** | In progress | Bridge code exists and works in isolation; not released as a packaged embedder yet. |
| **.NET / WinUI** | In progress | Same state as Kotlin — working bridge, no release yet. |
| **Edge** | Planned, not started | No code. |
| **Cloud placement** | Explicit non-goal for v0.1 | This is a deliberate scope decision in the governing specs, not an oversight — don't imply it's coming soon. |

## What this means for a capability you just built

- A contract with `permitted_targets: ["local"]` is honest for almost every new capability today. Widening it to `browser` is honest too, once you've actually built and tested the browser embedder path — don't add a target to the list just because the enum allows it.
- Setting `permitted_targets` to all six values (the deserializer's default if you omit the field) is a governance-level claim, not a technical one — the schema will accept it, but it doesn't mean the capability can actually run on `edge` or `cloud` today. Don't let the default stand in for a decision.
- If a user asks "can I ship this to iOS," the honest answer is no, and it's not a Traverse packaging problem — it's a blocked upstream dependency (no compliant WASM engine exists yet for that platform). Say that plainly rather than suggesting a workaround.
- MCP exposure (`traverse-mcp`) is a separate axis from placement targets — it's a stdio server with 9 tools (describe_server, list_content_groups, describe_content_group, list_entrypoints, describe_entrypoint, validate_entrypoint, execute_entrypoint, render_execution_report, shutdown) that lets an agent discover → inspect → execute → trace a capability. It works today over stdio only; there's no library API yet.

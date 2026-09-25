# Upgrading to Traverse v0.13.0

Governed by Spec `139-embedder-app-state-machine-execution`, Spec
`140-host-authority-wit-adapters`, ADR-0075, ADR-0076, Decision 96, Decision
97, Decision 99. See [docs/releases/v0.13.0.md](releases/v0.13.0.md) for the
full release narrative.

## Exact package versions

| Channel | Package | Version |
|---|---|---|
| crates.io | `traverse-runtime`, `traverse-embedder`, `traverse-contracts`, `traverse-mcp`, `traverse-cli-rs`, `traverse-expedition-wasm` | `0.13.0` |
| npm | `traverse-embedder-web` | `0.13.0` |
| Maven Central | `com.traverse-framework:traverse-embedder` | `0.13.0` |
| nuget.org | `TraverseEmbedder` | `0.13.0` |
| Swift (SwiftPM) | `packages/swift/TraverseEmbedder` binary target | `swift-host-v0.13.0-1` xcframework (see constraint below) |

All are lockstep per Decision 85 except the Swift binary target, which is
pinned by release tag rather than a semver package version (see
[docs/release-process.md](release-process.md)).

## embedder-api 1.1.0

Amends Specs `052-app-state-machine` (1.1.0), `057-embeddable-runtime-host`
(1.1.0), `059-http-command-dispatch` (1.1.0), and
`1402-runtime-wasm-orchestrator-convergence` (1.3.0). The application state
machine now executes **inside `runtime.wasm`** for standalone embedders (not
per-embedder hosts, not only via `traverse-cli serve`):

- `runtime.submit` accepts a typed `app_command` envelope (fail-closed
  discrimination on unknown commands); no new `runtime.command` verb.
- Host-connector waits use dual deadlines: the host may finish first, but the
  runtime enforces a hard ceiling via host-provided monotonic timer
  callbacks. The first correlated terminal event wins.
- SM sessions are **process-local only** — no crash/restart recovery yet
  (`#1470`, still `needs-spec`; do not build on durability that doesn't
  exist).
- Nested `runtime.wasm` memory ceiling raised to 32 MiB (from 16 MiB),
  matching native `WasmExecutor`; `runtime.wasm` was recertified against
  this, but the digest itself is unchanged from `v0.12.0`
  (`a254c161d5699b19ffe6eb47d2e11e45a39363ae8b2a9ac435d54a09fb31d782`,
  bridge version `1.1.0` — see `runtime/native-runtime-registry.json`).

## Manifest changes

- `invoke.host_connector` and `invoke.capability_id` are **mutually
  exclusive** on an `invoke` state (Spec 052 FR amendment).
- Every state with `invoke.host_connector` **must declare both required
  unhappy routes** — failure and timeout — or `app validate` rejects it.
- Decision 99: `invoke.input_from` can resolve
  `host_connector_result.<field>` at runtime, so a step can consume another
  host connector's own artifact without an authority field on the payload.
  Needs Spec 139 `0.2.0` / Spec 138 `0.3.0` or later. `app validate` rejects
  an unreachable reference.

## Host authority / adapter registration (Spec 140)

Host authorities are now target-neutral, WIT-defined, and runtime-owned
(Decision 97) rather than native-only. The first authority on this pattern is
`traverse.audio-input`. A registry-published host binding never
self-activates — the app or embedder must explicitly register an adapter per
platform:

```kotlin
// Kotlin
embedder.registerHostConnectorAdapter(command, adapter)
```

Equivalent registration exists on Swift, .NET, and web embedders. There is no
default or automatic audio-input binding; omitting registration means that
authority is simply unavailable to the app, not silently substituted.

## Bundle layout

Unchanged from `v0.11.0`/`v0.12.0`: apps still ship `runtime/runtime.wasm` +
`.sha256` in the app bundle; embedders still load and digest-verify it from
there. No bundle-layout migration is needed for this release.

## Known integration constraints

- **Use `swift-host-v0.13.0-1`, not `swift-host-v0.13.0`.** The first Swift
  XCFramework publish for this release predated a real fix (#1557): a
  missing `wasmi` `portable-dispatch` feature meant a non-terminating guest
  could crash the host with a native stack overflow instead of failing
  closed. `swift-host-v0.13.0` is immutable and cannot be corrected in
  place, so the fixed artifact ships under `swift-host-v0.13.0-1` instead.
  `packages/swift/TraverseEmbedder/Package.swift` already points at the
  corrected tag as of `main`.
- **There is no GitHub Release object named `v0.13.0`.** This repo's
  immutable-releases policy blocks publishing one after the first (empty)
  attempt during this cut. crates.io, npm, Maven Central, and nuget.org all
  consume their own registries directly and were unaffected; only the Swift
  binary distribution needed a release object, hence the dedicated
  `swift-host-v*` tag namespace. See
  [docs/releases/v0.13.0.md](releases/v0.13.0.md) and
  [Discussion #1552](https://github.com/orgs/traverse-framework/discussions/1552).
- **Durable SM session recovery does not exist yet.** A process restart
  loses in-flight application state-machine sessions and saga correlation
  state. Track `#1470` if your app needs crash recovery for long-running
  capture/analysis flows.
- **Audio-input is opt-in per platform.** `traverse.audio-input` requires an
  explicit adapter registration on each target you support; there is no
  cross-platform default.

## Verification

```bash
cargo test -p traverse-runtime --test app_state_machine_conformance
bash scripts/ci/native_artifact_certification.sh
```

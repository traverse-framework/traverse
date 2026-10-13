# Upgrading to Traverse v0.15.0

Governed by Spec `138-governed-exact-model-execution` (0.13.4) and
Decisions 104–114. See [docs/releases/v0.15.0.md](releases/v0.15.0.md) for
the full release narrative.

## Exact package versions

| Channel | Package | Version |
|---|---|---|
| crates.io | `traverse-runtime`, `traverse-embedder`, `traverse-contracts`, `traverse-mcp`, `traverse-cli-rs`, `traverse-expedition-wasm` | `0.15.0` |
| npm | `traverse-embedder-web` | `0.15.0` |
| Maven Central | `com.traverse-framework:traverse-embedder` | `0.15.0` |
| nuget.org | `TraverseEmbedder` | `0.15.0` |
| SwiftPM | `TraverseEmbedder` (binary target on `swift-host-v0.15.0`) | `0.15.0` |

These are lockstep per Decision 85. `traverse-registry` moves to `0.28.0`.

## Breaking: model rights enforcement (Spec 138 0.8.0, Decision 107)

Only apps that use `traverse.model-runtime` / `model.execute` exact-ref
models are affected.

1. **Declare `model_usage`.** An app with `exact_model_dependencies` must
   declare `model_usage: commercial | non_commercial`. Until it does,
   registration and execution fail with `model_incompatible` /
   `usage_undeclared`.
   - Rust: set `ExactModelHostConnector::model_usage`.
   - Web: pass `modelUsage` in the `ExactModelBrowserHost` options.
   - Swift: `ExactModelHost(pins:trustedPublicKeysHex:modelUsage:…)`. The
     argument has no default, so existing call sites stop compiling until
     you add it.
2. **Commercial use is checked.** A `commercial` usage (from the app, or
   forced by the host's `host_requires_commercial`) rejects packages whose
   rights say `prohibited` (`rights_policy_denied`).
3. **Browser offline pins.** A pin with `offline_allowed: false` now fails
   in the browser with `model_unavailable`, because the browser is always
   cache-only. This matches native offline mode.

## Breaking: Rust API additions

These are additive on the wire, but they break code that builds structs
with literals or matches enums exhaustively:

- `ExecutionPolicy` has new fields `placements: Vec<String>` (empty means
  `["wasm-cpu"]`) and `require_fuel_metering: bool`.
- `HostConnectorHostResult` has new fields `model_evidence` and
  `placement_evidence`.
- `ModelFailureReason` has new variants: `usage_undeclared`,
  `rights_policy_denied`, `rights_inconsistent`, `package_revoked`,
  `host_limit_exceeded` and `placement_unavailable`. Add a wildcard arm, or
  handle each one. (The Kotlin and .NET hosts also report
  `engine_unavailable` when their native library cannot load.)

## Unchanged and additive

- Manifest schemas `2.0.0`, `2.1.0` (`rights.derivation`) and `2.2.0`
  (`max_prepare_fuel`, `accelerator_variants`) all verify. Packages signed
  for v0.14.0 keep working: re-signing is not required.
- Guest ABI v1 guests are unchanged. ABI v2 (`model_alloc`) and ABI v3
  (`model_prepare` with snapshot reuse) are opt-in through the manifest's
  `abi_version`.
- Accelerator placement is opt-in. Without `placements`, every call runs
  on `wasm-cpu` as before. Core ML runs only when a Swift host passes
  `coreML:` and the call lists `coreml`.
- The Swift, Kotlin and .NET `ExactModelHost` types are new APIs.

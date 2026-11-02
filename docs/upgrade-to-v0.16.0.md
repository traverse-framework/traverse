# Upgrading to Traverse v0.16.0

See [docs/releases/v0.16.0.md](releases/v0.16.0.md) for every change in this
release.

## Exact package versions

| Channel | Package | Version |
|---|---|---|
| crates.io | `traverse-runtime`, `traverse-embedder`, `traverse-contracts`, `traverse-mcp`, `traverse-cli-rs`, `traverse-expedition-wasm` | `0.16.0` |
| npm | `traverse-embedder-web` | `0.16.0` |
| Maven Central | `com.traverse-framework:traverse-embedder` | `0.16.0` |
| nuget.org | `TraverseEmbedder` | `0.16.0` |
| SwiftPM | `TraverseEmbedder` (binary target on `swift-host-v0.16.0`) | `0.16.0` |

These are lockstep per Decision 85. `traverse-registry` moves to `0.29.0`.
Building from source needs Rust 1.95.

## Breaking: model registration takes the policy (Spec 138 0.16.0, Decision 120)

Only apps that run exact-ref models are affected.

1. **Pass the policy at registration.** Give the registration call the same
   `{policy_ref, placements, require_fuel_metering}` your calls name:
   - Rust: `register_package` / `register_package_with_variants`;
   - Swift, Kotlin and .NET: pass an `ExactModelPlacementPolicy`;
   - web: `registerPackage(..., policy, options?)`;
   - framed protocol: `register` takes `policy`.
2. **Register every `policy_ref` you call.** A call naming a `policy_ref` the
   package was not registered under fails with `model_unavailable` /
   `placement_unavailable`.
3. **Expect slower registration and faster first calls.** The Core ML compile,
   the device self-check and `model_prepare` now run at registration. A failing
   `model_prepare` fails registration with `execution_failed`. Each placement's
   `preparation_budget_ms` is capped by `max_preparation_ms` (default five
   minutes).
4. **Stop passing placements on calls.** The call's `placements` and
   `require_fuel_metering` are optional and deprecated. When present they must
   equal the registered policy.

## Breaking: canonical guest input (Spec 1722, Decision 118)

Every host now gives a guest the RFC 8785 (JCS) canonical bytes of its JSON
input.

- Browser and Node guests now receive sorted keys and ECMAScript number
  formatting instead of the caller's bytes. A capability that hashes its input
  gets a different hash on those hosts.
- Input JCS cannot represent fails with `input_not_canonicalizable`: numbers
  with magnitude above 2^53 − 1, duplicate member names and invalid Unicode.
- Rust: `ExecutorError` gains `InputNotCanonicalizable`. Update exhaustive
  matches.

## Breaking: contract input schemas are enforced (`#1658`)

Inputs are validated against the contract's `inputs.schema` before the artifact
runs. Unknown fields under `additionalProperties: false` and strings over
`maxLength` fail with `invalid_input` / `RequestInvalid`. Remove any fields
your contract doesn't declare, or declare them.

## Breaking: Registry rights fail closed (Spec 045 1.1.0, `#1728`)

- Version ranges skip revoked versions. An exact pin to a revoked version
  resolves, then fails closed with the revocation record. Move the pin to a
  version that isn't revoked.
- Conditional model rights need an explicit host opt-in. Unknown rights are
  always denied.

## Other changes to check

- **ONNX runner.** Download `onnx-runner-v0.16.0`. Its digest changed, and
  `model package-onnx` from this release accepts only that runner.
- **`traverse-mcp` Mode A.** Use the [README quickstart](../README.md) to sync
  and prepare a verified cache with no Rust toolchain.

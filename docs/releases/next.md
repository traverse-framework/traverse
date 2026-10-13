# Next Release Notes

Unreleased changes since `v0.14.0` will be recorded here.

## Fix: host-configurable nested limits for `runtime.wasm` (`#1656`)

- Nested capability fuel / memory inside `runtime.wasm` stay defaulted to
  10M / 32 MiB, but hosts can raise them via `traverse_init` `limits` and
  `BundleEmbedder.init({ limits: { fuel, maxMemoryBytes } })`.
- Bundle load / native registration fail closed with
  `resource_requirements_exceed_host` when a capability's declared
  `execution.resources` or WASM initial memory exceeds the host ceiling.
- Spec 1402 FR-012 amended to 1.5.0; contract `execution.resources` is
  optional and backwards-compatible.

## Fix: `model package-onnx` docs/CLI match ABI v3 / schema 2.2.0 (`#1660`)

- Help and `docs/model-packaging-guide.md` now describe the ABI v3 /
  schema `2.2.0` packages the ONNX packager actually emits (including
  `max_prepare_fuel`).
- `package-onnx` accepts `--json` like the other `model` subcommands.
- `model verify --json` prints failure JSON on stdout (exit code still
  signals failure).
- Unsupported `schema_version` / `abi_version` registration errors name the
  unsupported value in the message and `detail`.

## Fix: web embedder release evidence reports the real package version (`#1654`)

- `traverse-embedder-web` no longer hard-codes `PACKAGE_VERSION` as `0.9.0`.
  The value is generated from `package.json` at build time, so
  `releaseEvidence().package.version` matches the published package.
- The web embedder publish workflow unpacks `npm pack` output and fails if
  the packed dist version differs from `package.json` / the `web-v*` tag.
- Native Swift / Kotlin / .NET embedders take `packageVersion` as an input to
  `TraverseReleaseEvidence` (no hard-coded package constant to drift).

## Breaking: model rights enforcement (Spec 138 0.8.0, Decision 107, `#1599`)

- **`model_usage` is required.** Apps with `exact_model_dependencies` must
  declare `model_usage: commercial | non_commercial`; without it,
  registration and execution fail closed with
  `model_incompatible` / `usage_undeclared`. Set it on
  `ExactModelHostConnector::model_usage` (Rust). In Swift, pass
  `ExactModelHost(pins:trustedPublicKeysHex:modelUsage:limits:)`; the new
  `modelUsage` argument has no default, so existing call sites must be
  updated.
- **Usage policy:** a `commercial` usage (from the app, or forced by the
  host's `host_requires_commercial`) rejects `prohibited` packages with
  `rights_policy_denied`.
- **New reasons:** `usage_undeclared`, `rights_policy_denied`,
  `rights_inconsistent` and `package_revoked`. Rights failures carry a
  structured `error.detail`, and the Spec 137 dispatch failure now preserves
  the adapter's `reason` and `detail`.
- **Package status:** a host-owned status map via
  `ExactModelHostConnector::set_package_status`. `revoked` blocks the next
  register or execute; `deprecated` runs but is flagged.
- **Rights in evidence:** every `model.execute` result and dispatch document
  carries `model_evidence`, the verified rights record. The
  `traverse.model-runtime` connector contract is now `2.1.0` (additive).
- **Manifest schema `2.1.0`:** adds an optional `rights.derivation`; hosts
  accept `2.0.0` and `2.1.0`.
- **Shared rights conformance suite:**
  `fixtures/models/rights-conformance/suite.json`. Web (`#1600`) and Swift
  (`#1601`) parity follow.

### Web embedder parity (`#1600`)

- `ExactModelBrowserHost` enforces the same rights contract and passes the
  shared suite. **Breaking:** pass `modelUsage` in the host options when an
  app has model pins; otherwise registration fails with `usage_undeclared`.
- New: `hostRequiresCommercial`, `setPackageStatus`, `modelRightsRecord`,
  `effectiveUsage`, `ExactModelError.detail`, `model_evidence` on execute
  results and traces, and manifest schema `2.1.0` (`rights.derivation`).
- A pin with `offline_allowed: false` now fails with `model_unavailable` in
  the browser, which is always cache-only, matching native offline mode.

### Swift parity (`#1601`)

- `ExactModelHost` exposes the rights contract:
  - `hostRequiresCommercial:`;
  - `setPackageStatus` and `modelRightsRecord(digest:)`;
  - `ExactModelError.detail` and `ModelRights.derivation`;
  - `ExactModelExecution.modelEvidence`.

  The `modelExecuteAdapter` payloads also carry `model_evidence` and
  `detail`. The new framed ops are `rights_record` and `set_package_status`;
  the C symbol is unchanged.
- This needs a rebuilt `TraverseSwiftHost.xcframework`: a
  `swift-host-v0.14.0-2` release, after which `Package.swift` is repointed.
  Until then, the shared-suite Swift test fails against
  `swift-host-v0.14.0-1`.

### Kotlin/Android exact-ref model execution (`#1580`, Decision 108)

- The new `ExactModelHost` in `traverse-embedder` (Kotlin) runs signed
  Spec 138 packages on `wasmi` (SIMD) through the new Rust JNI shim
  `traverse-android-host` (ADR-0079). It shares the framed protocol with the
  Swift host through `traverse-model-host-frame`.
- It passes the signed vectors byte-for-byte (classifier, digits-mlp,
  digits-onnx) and the 21-case rights conformance suite.
- If the native library can't load, model calls fail with
  `model_unavailable` / `engine_unavailable` (new reason).
- Kotlin unit tests now need a Rust toolchain (`testDebugUnitTest` builds the
  host-JVM library). Packaging the `arm64-v8a` / `x86_64` `.so` files into
  the AAR is the next slice.

### Android AAR ships the native model engine (`#1580`, Decision 108)

- The `traverse-embedder` AAR now carries `libtraverse_android_host.so` for
  `arm64-v8a` and `x86_64`. It is cross-built with `cargo-ndk` at API 28 and
  16 KB page-aligned, as Android 15+ requires.
- A PR CI job (`android-native-package`) builds both, checks that each
  exports exactly the audited JNI method, and verifies the release AAR
  contains them.

# Exact-ref model execution (Spec 138)

Owner-approved Decisions 91–92. Public invoke remains Spec 137
`dispatch_host_connector_command` → `traverse.model-runtime` /
`model.execute`.

## Surfaces

| Surface | Role |
| --- | --- |
| App `exact_model_dependencies` | Exact pins (`model_id`, `version`, `digest`, `target`, `rights`, optional `key_id`) |
| `stage_model_input` / `read_model_output` | Host embedder I/O (opaque refs) |
| `stage_artifact` / `read_artifact` | Generic bounded artifacts (Spec 140): opaque, multi-read until drop or shutdown; runtime `ModelIoStore`, web `ModelIoStore`, Swift `ArtifactStagingStore` |
| `model.execute` | Command port; must-match `model_ref`, `policy_ref`, `data_classification` |
| `ExactModelHostConnector` | Native wasm-cpu adapter (`traverse_runtime::exact_model`) |
| `register_package` / `registerPackage` | Signed package admission into the host cache (Decision 101) |
| Versioned schemas | `contracts/connectors/traverse.model-runtime/schemas/`: `exact-model-pin-2.0.0`, `model-package-manifest-2.0.0`, `model-package-signature-1.0.0` |
| `model_rights` / `modelRights` | Signed rights, read-only, for host/UI display |
| Fixtures | `fixtures/models/fixture-{echo,classifier,responder}-1.0.0/` (signed, test-only key), `fixtures/models/conformance/signed-classifier.json` |
| `input_from: host_connector_result.<field>` | Spec 139 app-state-machine capability step resolved through the FR-017 runtime-mediated path above (Decision 99) |

## Placement

First conformance profile: `wasm-cpu` (import-denied guest, LE frames).
Native (`ExactModelHostConnector`) and browser (`ExactModelBrowserHost`)
run the same signed package and produce byte-identical output for the
checked-in conformance vector.

## Signed packages and exact-ref resolution (Spec 138 0.4.0, Decision 101)

A package is three files:

| File | Content |
| --- | --- |
| `model.manifest.json` | Schema `2.0.0`: identity, `wasm_digest`, ABI, schema refs, limits, `supported_profiles`, and a required `rights` object |
| `model.wasm` | Import-free guest exporting `memory` and `model_execute` |
| `model.sig.json` | `{ "alg": "ed25519", "key_id", "signature" }`: hex Ed25519 signature over the **exact** manifest bytes |

An app pin's `digest` is the SHA-256 of the exact manifest bytes, so one hash
binds identity, rights, limits, and (through `wasm_digest`) the WASM. The
runtime resolves **only** that exact reference: there is no download,
version range, or substitution during execute.

**Trust is host-owned.** The host configures trusted Ed25519 public keys
(`TrustedModelKeys` natively, `trustedPublicKeysHex` in the browser). An app
manifest cannot add a key; a pin may only narrow to one trusted `key_id`
(`ed25519:` + hex SHA-256 of the raw public key).

**Registration** (cache admission) verifies the signature, the trusted key,
that the manifest digest equals exactly one pin, the manifest schema (unknown
fields fail closed), identity, rights completeness, the target, that the pin's
`rights.license_id` / `rights.commercial_use` equal the signed rights, and the
WASM digest. Each execute re-hashes the cached bytes against the pin.

Failures keep `model_unavailable` / `model_incompatible` and add a stable
`reason`: `pin_mismatch`, `pin_ambiguous`, `signature_invalid`,
`key_untrusted`, `digest_mismatch`, `manifest_invalid`, `rights_incomplete`,
`rights_mismatch`, `target_unsupported`, `crypto_unavailable`,
`candidate_unsupported`, `host_limit_exceeded`.

## Engines, host ceilings, and interruption (Spec 138 0.6.0, Decision 104)

| Host | Engine | Host ceilings | Mid-run cancel/timeout |
| --- | --- | --- | --- |
| Rust native (`ExactModelHostConnector`) | wasmtime (default) or `wasmi` (`ModelEngine::Wasmi`) | yes (`HostModelLimits`) | `wasmi`: yes; wasmtime: #1582 |
| Swift host (`traverse_swift_host_model_call`, ADR-0078) | `wasmi` only (iOS forbids JIT) | yes (supplied at `create`) | yes (fuel slices) |
| Web (`ExactModelBrowserHost`) | browser WebAssembly | #1582 | #1582 |

- **Host ceilings:** a host caps package bytes, guest memory, and fuel.
  Registration fails with `host_limit_exceeded` when a package's size or
  declared limits exceed them, and execution uses manifest ∩ host ∩
  per-call.
- **`wasmi` interruption:** `wasmi` runs the guest in fuel slices
  (`WASMI_FUEL_SLICE`). Between slices it checks a caller-managed cancel
  flag (→ `cancelled`) and the deadline (→ `timeout`). The Swift host
  cancels only the named `execution_id`, so a late cancel never affects the
  next execution.
- **Fuel is engine-relative:** each engine counts `max_fuel` in its own
  units. Size it so the package's conformance vector passes on every
  engine; `max_execution_ms` is the portable bound.
- **Swift API:** `ExactModelHost` on the Swift embedder ships once an
  xcframework containing the sixth symbol is published (`#1579`,
  follow-up).

## Cache and offline behavior

Verification is purely local: SHA-256 plus Ed25519. Once a package is
registered, validation, activation, and execution make **zero network
calls** (the browser test stubs `fetch` and `XMLHttpRequest` and asserts no
calls). A cache miss is `model_unavailable`. The host owns provisioning, i.e.
how the three files reach the device; the runtime never fetches them.

## Rights propagation

The signed `rights` object (`license_id`, `attribution`, `redistribution`,
`commercial_use`, `source_url`) is exposed to the host unchanged via
`model_rights` / `modelRights`, so a UI can show attribution and
commercial-use terms without reimplementing policy. The model bytes and
execution never reach the UI; it receives runtime state and typed results
only. `source_url` is informational: identity is always the digest.

## Browser/native portability boundary

- Browser verification uses WebCrypto Ed25519 only (Chrome 137+, Firefox
  129+, Safari 17+, Node 20+). Without it, registration fails closed with
  `crypto_unavailable`. There is no pure-JS fallback.
- Browser resolution is **single exact-ref only**: every pin must target
  `wasm-cpu`; anything else (for example an Ollama candidate) fails with
  `candidate_unsupported`. Mixed-candidate resolution in the browser is
  `#1460`.
- Fuel metering exists only natively (wasmtime). The browser enforces the
  memory, input, and output ceilings, a post-hoc timeout, and `AbortSignal`
  cancellation.
- The browser `execute` returns a typed result with `model_ref`, `target`,
  and a redacted `trace` (identity, digest, placement, classification, usage).

## Fixtures

`node scripts/fixtures/sign-model-fixtures.mjs` regenerates the manifests,
signatures, and conformance vectors deterministically with the **test-only**
key in `fixtures/models/test-signing-key.json`. No host trusts that key by
default; production signing is `#1567`. `echo`, `classifier`, and `responder`
are hand-written deterministic fixtures. `digits-mlp-1.0.0` is a real trained
model (below).

## Trained model: `digits-mlp-1.0.0` (Spec 138 0.5.0, Decision 102)

A 64 → 32 (ReLU) → 10 MLP (2,410 `f32` weights) trained on the UCI Optical
Recognition of Handwritten Digits dataset (CC BY 4.0). It scores **96.10%
(1,727 / 1,797)** on the held-out test split, bit-identically in the host
trainer, the native wasmtime host, and the browser host.

| Piece | Location |
| --- | --- |
| Data (vendored, pinned, CC BY 4.0) | `fixtures/datasets/uci-optdigits/` (`ATTRIBUTION.md`, `SHA256SUMS`) |
| Trainer (seeded, offline) | `crates/traverse-model-trainer` → `cargo run --release -p traverse-model-trainer` |
| Weights (pinned) | `crates/traverse-digits-mlp-guest/weights/digits-mlp-1.0.0.bin{,.sha256}` |
| Guest (`no_std` on wasm32, audited ABI boundary per ADR-0077) | `crates/traverse-digits-mlp-guest` (its own `[workspace]`) |
| Signed package | `fixtures/models/digits-mlp-1.0.0/` |
| Conformance vector | `fixtures/models/conformance/signed-digits-mlp.json` |

**Frames.** The input is dtype `2` (`f32`), dims `[64]`: the 8×8 raw pixel
counts `0..=16`, row-major. The output is dtype `3` (`f32`), dims `[11]`: 10
logits followed by the predicted class. There's no softmax, so results are
bit-identical across hosts. Malformed frames or out-of-range pixels return
`-1`, and the host fails closed.

**Limits** (from measured usage): about 56k fuel per inference (ceiling
200,000), one 64 KiB memory page (ceiling 128 KiB), a 268-byte input, and a
56-byte output (ceiling 64).

**Rights.** `license_id` `CC-BY-4.0`, `commercial_use` `allowed`, and an
`attribution` crediting E. Alpaydin & C. Kaynak (UCI,
doi:10.24432/C50P49). A UI must show that attribution.

**Retraining.** Retraining changes the weights, so every downstream artifact
must be regenerated in order:

1. `cargo run --release -p traverse-model-trainer` rewrites the weights and
   their digest.
2. `cargo build --release --target wasm32-unknown-unknown` in the guest
   crate, then copy the `.wasm` to `fixtures/models/digits-mlp-1.0.0/model.wasm`.
3. `node scripts/fixtures/sign-model-fixtures.mjs` re-signs and regenerates
   the vector.
4. Update the pinned accuracy count in the tests.

`scripts/ci/digits_mlp_guest_check.sh` fails CI if the checked-in
`model.wasm` no longer rebuilds byte-identically.

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
`candidate_unsupported`.

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
signatures, and conformance vector deterministically with the **test-only**
key in `fixtures/models/test-signing-key.json`. No host trusts that key by
default. The fixtures are hand-written deterministic guests. A real,
licensed trained model package is `#1461`.

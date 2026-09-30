# Feature Specification: Governed Exact-Ref Model Execution

**Feature Branch**: `codex/issue-1435-governed-exact-model-execution`
**Created**: 2026-09-16
**Status**: Approved (2026-09-16)
**Canonical governing ID**: `138-governed-exact-model-execution`
**Version**: 0.7.0
**Extends**: `137-host-connector-command-dispatch`,
`044-application-bundle-manifest`, `526-embedded-verified-cache-lifecycle`,
`1259-portable-authority-contracts`, and Registry signed-artifact verification.
**Amendment (2026-09-18, version 0.1.0 -> 0.2.0, approved 2026-09-18)**: Decision 97 /
Spec `140-host-authority-wit-adapters`. Host staging is generalized from
model input/output to bounded host artifacts, including audio produced by
`traverse.audio-input`. Model-named APIs remain valid.
**Amendment (2026-09-21, version 0.2.0 -> 0.3.0, approved 2026-09-21)**: Decision 99.
Formalizes the previously descriptive "runtime resolves an `artifact_ref`
through staging" behavior as FR-017. Spec 139 defines the app-state-machine
`input_from` syntax (`host_connector_result.<field>`) that is its first
caller. Unblocks `#1502` / `#1503`.
**Amendment (2026-09-28, version 0.3.0 -> 0.4.0, approved 2026-09-28)**: Decision 101 /
`#1565`. Defines the signed-package trust model the spec previously only
named ("verify Registry signature"): host-owned Ed25519 trust roots, a
detached signature over the exact manifest bytes, pin digest = SHA-256 of
those bytes, a nested required `rights` object (adds `commercial_use` and
`source_url`), app-declared rights checked at registration, stable failure
`reason` values, WebCrypto-only browser verification, and an explicit
single exact-ref browser resolution boundary (FR-018 through FR-023).
Manifest `schema_version` becomes `2.0.0` (breaking; `package_digest` and the
flat license fields are removed).
**Amendment (2026-09-29, version 0.4.0 -> 0.5.0, approved 2026-09-29)**: Decision 102 /
ADR-0077 / `#1461`. Adds the first trained exact-ref package and the
provenance rules any trained package must meet: vendored, digest-pinned,
licensed training data; a deterministic in-repo trainer; committed pinned
weights; a guest whose checked-in `model.wasm` rebuilds byte-identically in
CI; and a held-out accuracy floor enforced through the signed
register → execute path on both native and browser (FR-024 through FR-027).
Additive; no ABI or schema change.
**Amendment (2026-09-29, version 0.5.0 -> 0.6.0, approved 2026-09-29)**: Decision 104 /
ADR-0078 / `#1579`. Makes three rules project-wide:
- host ceilings, with a new additive reason `host_limit_exceeded`;
- mid-run interruption for cancellation and deadlines;
- engine-relative fuel proven by conformance.
Adds `wasmi` as a supported `wasm-cpu` engine for JIT-forbidden targets
(FR-028 through FR-031). Additive; no manifest schema change.
**Amendment (2026-09-30, version 0.6.0 -> 0.7.0, approved 2026-09-30)**: Decision 105 /
`#1588`. Adds **guest ABI v2**: manifest `abi_version: 2` makes the host
obtain input and output buffers from the guest's `model_alloc(len) -> ptr`
instead of fixed offsets, so guests with a real heap (for example an ONNX
runner) are safe. v1 is unchanged (FR-032 through FR-034).

**Decision evidence**: Decision 91; Decision 92; ADR-0074 (Accepted).
**Input**: Callweave portable governed model-execution slice request
(`MODEL_EXECUTION_SLICE_REQUEST.md`).
**Approval**: Owner-approved in session 2026-09-16 (Decisions 91–92 + explicit
approval to implement).

## Purpose and boundary

Define the portable Traverse contract for invoking an **exact, signed model
package** declared by an application manifest: resolve and verify the package,
execute it under resource limits on a provider-neutral executor, and return a
typed inference result with redacted trace evidence.

The public app-runtime **command** port is Spec 137 `model.execute` on
`traverse.model-runtime`. Binary tensors cross that port only as opaque
`input_ref` / `output_ref` handles. Host embedders expose
`stage_model_input` and `read_model_output` outside Spec 137 command kinds.

This is **not** Spec 045 (candidate sets, Ollama, fallback). Spec 045 remains
the LLM/candidate track and MUST NOT be treated as satisfying this DoD.

## Relationship to existing specifications

| Specification | Relationship |
| --- | --- |
| 137-host-connector-command-dispatch | Public command port. This spec defines `model.execute` inference fields and guest/cache semantics. |
| 044-application-bundle-manifest | Normative `exact_model_dependencies` pin array home. |
| 045-governed-model-dependency-resolution | Separate LLM/candidate track; non-normative cross-pointer only. |
| 526-embedded-verified-cache-lifecycle | Model **packages** provision into verified cache generations. Ephemeral tensor refs are not Spec 526 entries. |
| 1259 / ADR-0060 | `traverse.model-runtime` remains vendor-neutral authority. |
| 104 / 135 | Guest `connector_invoke` and Component WIT fakes are out of this surface. |
| 139-embedder-app-state-machine-execution | First caller of FR-017's runtime-mediated resolution via `input_from: host_connector_result.<field>` (Spec 139 FR-019/FR-020) |

## Model

1. Registry publishes a **model package**: sidecar `model.manifest.json` +
   `model.wasm`, bound by an explicit package/pair digest and signatures.
2. The application manifest declares `exact_model_dependencies` pins and an
   activated `traverse.model-runtime` binding (plus execution `policy_ref`
   objects as required by the connector binding).
3. Host provisioning uses Spec 526 `prepare` / `activate` for model packages.
   Execution uses only the **active** generation entry matching the pin.
4. Caller stages input bytes via host `stage_model_input` → opaque
   `input_ref` (single-consume).
5. App command routes to `model.execute` with must-match `model_ref`,
   `input_ref`, `policy_ref`, required `data_classification`, schema refs,
   JSON shape/dtype metadata, and per-call ceilings.
6. Host validates (fail closed on unknown fields), verifies
   digest/signature against the active cache entry, enforces
   manifest ∩ policy ∩ per-call limits, invokes the sandboxed guest.
7. Guest runs a versioned little-endian binary feature frame → output frame
   with **no** filesystem or network imports.
8. Host returns Spec 137 result with opaque `output_ref`, identity,
   placement (`wasm-cpu`), usage, redacted evidence, and status. Caller
   reads bytes via `read_model_output`.

## Exact model pins (app manifest)

Each `exact_model_dependencies` entry MUST include at least:

- `model_id`, semantic `version`, and `digest` = SHA-256 of the exact signed
  `model.manifest.json` bytes (0.4.0; this one hash binds manifest, rights,
  limits, and transitively the WASM via `wasm_digest`);
- Registry reference;
- whether offline execution is allowed after provisioning;
- `target` (execution profile; `wasm-cpu` is the only conformance target);
- `rights`: `{ license_id, commercial_use }` the signed package MUST carry;
- optional `key_id` narrowing to one host-trusted signer. A pin can never
  add trust.

Unknown pin fields fail closed. More than one pin with the same `model_id`
and `version` is ambiguous and fails closed.

`model.execute.model_ref` MUST equal a declared pin or fail closed
(`model_unavailable` / `model_incompatible` as appropriate).

## Model artifact manifest

Every executable model package MUST include a versioned manifest with at
least:

- `model_id` and semantic `version`;
- immutable content digests for manifest, WASM, and package/pair;
- Registry reference;
- executable format and model-ABI version;
- input and output schema references + schema versions;
- `rights` object (0.4.0), every field required and non-empty:
  `license_id` (SPDX), `attribution`, `redistribution`,
  `commercial_use` (`allowed` | `restricted` | `prohibited`), `source_url`;
- supported target profiles (at least `wasm-cpu` for conformance);
- maximum linear memory, fuel/instruction, input bytes, output bytes, and
  execution time;
- optional quantization / numeric precision metadata;
- provenance / source revision and build reproducibility evidence;
- whether offline execution is allowed after provisioning.

Validation MUST reject missing rights, digest, ABI, schema refs, or
resource limits, and any `schema_version` other than `2.0.0`. A model URL
alone (including `rights.source_url`) is never an acceptable identity.
Unknown fields fail closed.

## Signed package verification (0.4.0, Decision 101)

A package is `model.manifest.json` + `model.wasm` + `model.sig.json`.
`model.sig.json` is `{ "alg": "ed25519", "key_id", "signature" }` where
`signature` is the lowercase hex Ed25519 signature over the **exact**
manifest bytes (no JSON canonicalization) and `key_id` is `ed25519:` +
lowercase hex SHA-256 of the raw 32-byte public key.

Trust roots are **host-owned**: the embedder is configured with the set of
trusted public keys. Application manifests never add trust.

Registration (host cache admission / activation) verifies, failing closed:
signature document shape and algorithm; signer key is host-trusted and, when
the pin names `key_id`, equal to it; signature over the manifest bytes;
manifest digest equals exactly one pin; manifest parses with no unknown
fields; identity equals the pin; manifest validation (rights, limits,
schema version, `wasm-cpu`); pin `target` supported; pin `rights` equal the
signed `rights.license_id` and `rights.commercial_use`; WASM bytes match
`wasm_digest`. Every `model.execute` re-hashes the cached manifest and WASM
bytes against the pin. All verification is local; none of it uses the
network.

Signed `rights` are exposed read-only to the host/application unchanged so a
UI can show attribution and commercial-use terms without reimplementing
policy.

### Failure reasons

Failures keep the public codes `model_unavailable` / `model_incompatible`
and add a stable `reason`: `pin_mismatch`, `pin_ambiguous`,
`signature_invalid`, `key_untrusted`, `digest_mismatch`, `manifest_invalid`,
`rights_incomplete`, `rights_mismatch`, `target_unsupported`,
`crypto_unavailable`, `candidate_unsupported`, `host_limit_exceeded` (0.6.0).

### Browser embedder boundary

The browser embedder verifies with WebCrypto Ed25519 only; an environment
without it fails activation with `model_unavailable` /
`crypto_unavailable` (no pure-JS fallback, no host-injected verifier). It
accepts only single, already-selected `exact-ref` `wasm-cpu` pins; any other
candidate kind fails closed with `candidate_unsupported`. Mixed-candidate
(Spec 045) browser resolution is the future extension tracked by `#1460`.

## Host staging APIs (embedder surface)

- `stage_model_input(bytes, limits) -> input_ref`
- `read_model_output(output_ref) -> bytes` (size-capped)
- Optional explicit drop APIs MAY exist; shutdown MUST invalidate refs.

### Generalized bounded artifacts (Spec 140)

- `stage_artifact(bytes, limits) -> artifact_ref` and
  `read_artifact(artifact_ref) -> bytes` (size-capped) are the generic
  spellings; `stage_model_input` / `read_model_output` remain for models.
- A host adapter (for example `traverse.audio-input`) MAY stage its own
  result and return the opaque `artifact_ref`; the ref is not a path or URL.
- The runtime resolves an `artifact_ref` to a bounded capability input
  **only through runtime-mediated staging**; guests never read host storage.
- An adapter-produced `artifact_ref` is **multi-read** until explicit drop,
  host TTL, or runtime shutdown, so runtime-owned retries can re-read it.
  Model `input_ref` keeps its single-consume rule below.
- Resolution is size-bounded by the lesser of the artifact's originally
  staged ceiling and the consuming capability's declared input limit;
  exceeding it fails closed with `input_limit_exceeded` before the
  capability runs (FR-017; Decision 99). This governs the state-machine
  `input_from: host_connector_result.<field>` path (Spec 139 FR-020) and any
  other caller of runtime-mediated artifact resolution.

**Lifetime:**

- `input_ref`: valid until successful `model.execute`, cancel, or drop;
  **single-consume**.
- `output_ref`: readable until host TTL, explicit drop, or runtime shutdown.
- Neither ref is a Registry artifact or Spec 526 generation entry.

## Guest ABI (CPU-WASM baseline)

- Placement for first conformance: `wasm-cpu`.
- Guest export consumes/produces a **versioned little-endian length-prefixed**
  frame: `abi_version`, `dtype`, `rank`, `dims[]`, `payload_len`, payload.
- Guest MUST NOT receive host envelopes, credentials, paths, or network.
- Deny-by-default: no filesystem or network imports.
- Later accelerators (SIMD, WebGPU, Metal, Core ML, native ML) are
  policy-selected adapters that MUST preserve public envelopes, limits, and
  failure codes; detect independently; fail or fall back explicitly.
- Large models MAY declare a target unsupported rather than OOM or silent
  degrade.

### Guest ABI v2 (0.7.0, Decision 105)

- The manifest `abi_version` selects buffer placement. `1` means the host
  writes input at offset 64 and reserves output after it (the v1 behaviour
  above). `2` means the guest exports `model_alloc(len: i32) -> i32`, and the
  host calls it once for the input length and once for the output capacity,
  writes the input frame there, and passes both regions to `model_execute`.
- Returned regions MUST be positive, lie inside the guest's current linear
  memory, and not overlap. Otherwise the host fails closed with
  `execution_failed`. A missing `model_alloc` is `model_incompatible`; a
  trapping or fuel-exhausted `model_alloc` is `execution_failed`.
- The guest grows its own memory. The host still enforces the memory ceiling
  (engine limiter, or a post-allocation size check where there is none)
  → `resource_exhausted`.
- The little-endian frame format (frame `abi_version` field `1`) is
  unchanged. `abi_version` values above 2 are `manifest_invalid`.

## `model.execute` request payload (Spec 137 command `payload`)

Required:

- `model_ref`: `{ model_id, version, digest }` must-matching an app pin;
- `input_ref`: opaque host-staged handle;
- `policy_ref`: opaque execution-policy handle (ceilings, redaction profile,
  allowed classifications)—**not** model identity;
- `data_classification`: required per-call classification; policy may deny;
- `input_schema_ref` + schema version;
- JSON metadata for shape, dtype, layout, and applicable audio fields;
- per-call ceilings including `max_output_bytes`, each ≤ manifest and policy;
- cancellation/deadline as applicable (with Spec 137 `cancel_requested`).

Forbidden: `provider`, `endpoint`, `credential`, free model selection, and
unbounded JSON base64 tensors.

Unknown fields MUST fail closed. Spec 137 `idempotency_key` governs replay
(original result without second adapter invoke).

## Response semantics

Success/failure on the Spec 137 result path MUST surface:

- `status`: `ok`, `invalid_input`, `model_unavailable`,
  `model_incompatible`, `resource_exhausted`, `cancelled`, `timeout`, or
  `execution_failed`;
- `output_ref` and `output_schema_ref` on success (always opaque; never
  inline tensor bytes on the command result);
- exact model identity and digest;
- placement (`wasm-cpu` initially);
- trace ID and redacted evidence;
- measured resource usage;
- stable reason code, safe diagnostic message, and retryable flag on failure.

MUST NEVER expose credentials, host paths, private URLs, or internal runtime
details.

## Trained model packages (0.5.0, Decision 102)

A package that claims to be a trained model (not fixture logic) MUST ship
reproducible provenance alongside its signed files:

- **Data**: training and held-out data vendored in the repo with its
  licence and attribution, pinned by SHA-256, and never downloaded by
  training or CI.
- **Trainer**: a deterministic, seeded, offline trainer in the Cargo
  workspace. The committed weights are pinned by SHA-256 and consumed by the
  guest at build time.
- **Guest**: the checked-in `model.wasm` MUST rebuild byte-identically from
  the reviewed guest source with the pinned toolchain in CI, and MUST import
  nothing. A Rust guest's pointer-passing ABI boundary is an audited
  `unsafe` exception (ADR-0077).
- **Evidence**: CI enforces a declared held-out accuracy floor by running
  the signed package through `register_package` → `model.execute`
  natively, and the browser embedder MUST produce byte-identical output for
  a checked-in conformance vector.
- **Rights**: `rights` reflect the data licence (for example CC BY 4.0
  weights carry the dataset attribution).

The first such package is `fixtures/models/digits-mlp-1.0.0`: a
64 → 32 (ReLU) → 10 MLP trained on UCI Optical Recognition of Handwritten
Digits (CC BY 4.0) with a ≥ 95% held-out accuracy floor (96.10% measured).
It is signed with the test-only key; production signing is `#1567`.

## Resolver and cache behavior

Traverse MUST:

1. resolve only exact pins declared in `exact_model_dependencies`;
2. verify Registry signature and content digest before execution;
3. use Spec 526 content-addressed cache keyed by digest, not mutable URL;
4. support offline execution when the package is in the active generation and
   the manifest allows offline;
5. return stable `model_unavailable` / `model_incompatible` when absent or
   unsupported;
6. prevent download/replace/select outside the pin during execute;
7. keep package eviction/provisioning host-controlled and observable;
8. avoid network during offline validation or execution.

## Requirements

- **FR-001**: Define the versioned model package manifest; reject incomplete
  license/digest/ABI/schema/limit metadata; fail closed on unknown fields.
- **FR-002**: App manifests MUST declare `exact_model_dependencies`; runtime
  MUST reject non-matching `model_ref`.
- **FR-003**: `model.execute` is the only v1 public **command** invoke
  surface; guest `model_invoke` is out of scope.
- **FR-004**: Binary I/O MUST use host `stage_model_input` /
  `read_model_output` and opaque refs; not unbounded JSON base64.
- **FR-005**: First conformance executor MUST be `wasm-cpu` with deny-by-
  default FS/net for the model guest and the LE guest frame ABI.
- **FR-006**: Host MUST own public status, identity, placement, trace,
  usage, and retryable fields.
- **FR-007**: Model packages MUST use the host-owned verified digest cache
  (Spec 080 Mode A `HostRegistryCache` APIs for conformance today; Spec 526
  generation prepare/activate/rollback when that lifecycle is implemented on
  the same digest-keyed store). Ephemeral tensor refs MUST NOT be cache
  generation entries.
- **FR-008**: Offline warm-cache execute MUST succeed without network when
  allowed; miss → `model_unavailable`.
- **FR-009**: `policy_ref` MUST name execution policy only; required
  `data_classification` MUST be enforced against that policy.
- **FR-010**: Resource, cancellation, timeout, and output ceilings MUST be
  enforced.
- **FR-011**: Public traces MUST include model id, version, digest,
  placement, usage, and classification; MUST redact tensor bytes and secrets.
- **FR-012**: Acceleration adapters MUST preserve envelopes and failure codes.
- **FR-013**: Native CPU-WASM conformance is the first impl DoD; browser
  cross-target comparison is a same-spec follow-on.
- **FR-014**: No provider-specific API, second model registry, or
  Callweave-specific workflow behavior.
- **FR-015**: A signed example model package fixture MUST be publishable.
- **FR-016**: The `traverse.model-runtime` connector contract MUST be updated
  in the same governance approval as this spec (breaking schema bump).
- **FR-018**: Model packages MUST carry a detached Ed25519 signature over the
  exact manifest bytes; the pin `digest` MUST be SHA-256 of those bytes.
- **FR-019**: Signature trust roots MUST be host-owned; a pin MAY only narrow
  to one trusted `key_id` and MUST NOT add trust.
- **FR-020**: Registration MUST reject a signed package whose `rights`
  `license_id` or `commercial_use` differ from the pin (`rights_mismatch`)
  and MUST expose signed `rights` to the host unchanged.
- **FR-021**: Registration MUST perform the full verification in "Signed
  package verification"; every execute MUST re-check cached bytes against the
  pin digest (`digest_mismatch`).
- **FR-022**: Failures MUST carry the stable `reason` values listed above in
  addition to the public code.
- **FR-023**: The browser embedder MUST use WebCrypto Ed25519 and fail closed
  with `crypto_unavailable` when absent, and MUST accept only single exact-ref
  `wasm-cpu` pins (`candidate_unsupported` otherwise).
- **FR-024**: A trained model package MUST vendor its licensed training and
  held-out data pinned by SHA-256, and MUST NOT download data during training
  or CI.
- **FR-025**: A trained package's weights MUST come from a deterministic,
  seeded, in-repo trainer and be committed with a pinned SHA-256.
- **FR-026**: A trained package's checked-in `model.wasm` MUST rebuild
  byte-identically from its guest source in CI with the pinned toolchain,
  and MUST import nothing.
- **FR-027**: CI MUST enforce the package's declared held-out accuracy floor
  through the signed register → execute path, and native and browser MUST
  match a checked-in conformance vector byte-for-byte.
- **FR-028**: Every host MUST accept host-configured ceilings (package bytes,
  guest memory, fuel) with safe defaults. Registration MUST fail closed with
  `model_incompatible` / `host_limit_exceeded` when a package's size or
  declared limits exceed them, and execution MUST use
  manifest ∩ host ∩ per-call.
- **FR-029**: Hosts SHOULD interrupt a running inference mid-run on
  cancellation (`cancelled`) or deadline (`timeout`), not only before and
  after it. On `wasmi` this is done by fuel slices with resumable
  out-of-fuel calls, checking between slices. A cancellation MUST only
  affect the execution it names.
- **FR-030**: `max_fuel` is an engine-relative ceiling: each engine enforces
  it in its own units. A package's conformance vector MUST pass on every
  supported engine (wasmtime, `wasmi`, browser). `max_execution_ms` is the
  portable wall-clock bound.
- **FR-031**: On JIT-forbidden targets (iOS/macOS Swift host) the `wasm-cpu`
  guest MUST execute on `wasmi` behind the audited Swift-host ABI
  (ADR-0078), reusing the same verification as native hosts.
- **FR-032**: Hosts MUST support guest ABI v1 and v2, selected by the
  manifest `abi_version`, and MUST reject any other value as
  `manifest_invalid`.
- **FR-033**: For ABI v2, hosts MUST obtain both buffers from
  `model_alloc` and MUST reject non-positive, out-of-bounds, or overlapping
  regions, a missing `model_alloc`, or a trapping `model_alloc`, before
  writing any input.
- **FR-034**: A v2 conformance fixture MUST produce byte-identical output on
  wasmtime, `wasmi`, the browser, and the Swift host (through the shared
  Rust `wasmi` executor). v1 fixtures MUST be unaffected.
- **FR-017**: The runtime MUST resolve an `artifact_ref` into a bounded
  capability input only through runtime-mediated staging (`stage_artifact` /
  `read_artifact`); guests MUST NOT read host storage directly and MUST NOT
  receive a raw ref, a path, or a URL through this resolution. Resolution
  MUST fail closed with `input_limit_exceeded` when the artifact exceeds the
  lesser of its originally staged ceiling and the consuming capability's
  declared input limit, without invoking the capability (Decision 99).

## Acceptance scenarios

### Happy paths

1. Exact signed model resolves from Spec 526 active generation and executes
   on `wasm-cpu`.
2. Staged bounded input yields schema-valid output via `output_ref`.
3. Offline execution succeeds with a warm verified cache.
4. Trace identifies model id, version, digest, placement, usage, and
   classification.
5. (Follow-on) Acceleration reports the same public envelope as CPU baseline.
6. A Spec 139 capability step with `input_from:
   "host_connector_result.artifact_ref"` receives `{"artifact_base64": ...}`
   resolved via runtime-mediated staging, never a path, URL, or raw ref
   (FR-017).

7. (0.4.0) A signed package verifies against a host-trusted key and its
   rights are exposed to the host; native and browser produce byte-identical
   output for the checked-in signed conformance vector.
8. (0.4.0) Registration and execution with a cached package make zero
   network calls.
9. (0.5.0) The trained `digits-mlp-1.0.0` package verifies, and scores
   ≥ 95% on the 1,797-sample held-out split through signed native
   execution. The browser matches the native conformance vector
   byte-for-byte.

### Unhappy paths

1. Missing pin / version or digest mismatch.
2. Invalid or missing license or ABI metadata.
3. Unsupported schema, dtype, shape, or target profile.
4. Signature or digest failure.
5. Cache miss offline → `model_unavailable`.
6. Limit exceeded; cancel; timeout.
7. Unknown JSON keys fail closed; malformed guest frames fail closed.
8. Guest trap → `execution_failed`.
9. Guest FS/net attempt denied / fails closed.
10. `data_classification` denied by policy → `policy_denied`.
11. Re-use of consumed `input_ref` fails closed.
12. Retryable vs non-retryable classification is stable.
13. Runtime-mediated resolution of an artifact into a capability input
    exceeds the lesser of the artifact's staged ceiling or the capability's
    declared limit → `input_limit_exceeded` before invoke, capability never
    runs (FR-017).
14. (0.4.0) Unsigned, malformed, wrong-algorithm, or bad signature →
    `signature_invalid`; untrusted or pin-mismatched signer → `key_untrusted`.
15. (0.4.0) No or ambiguous pin → `pin_mismatch` / `pin_ambiguous`.
16. (0.4.0) Rights incomplete or differing from the pin →
    `rights_incomplete` / `rights_mismatch`.
17. (0.4.0) Unsupported pin target → `target_unsupported`; non-exact-ref
    browser candidate → `candidate_unsupported`; no WebCrypto Ed25519 →
    `crypto_unavailable`.
18. (0.4.0) Tampered cache bytes at execute → `digest_mismatch`.
19. (0.5.0) A trained guest given a malformed frame (wrong dtype, dims,
    length, or a pixel value outside `0..=16`) returns `-1` and the host
    fails closed. A per-call fuel ceiling below the measured need traps as
    `execution_failed`. A rebuilt guest whose bytes differ from the
    checked-in `model.wasm` fails CI.
20. (0.6.0) A package whose size or declared memory/fuel exceeds the host
    ceilings → `host_limit_exceeded` at registration; a long inference is
    interrupted mid-run by cancellation (`cancelled`) or deadline
    (`timeout`), and a stale cancellation never affects a later execution.
21. (0.7.0) A v2 guest whose `model_alloc` is missing, traps, or returns a
    zero, negative, out-of-bounds, or overlapping region fails closed before
    any input is written; `abi_version: 3` is `manifest_invalid`.

## Compatibility and non-goals

Non-goals: production animal-recognition model selection; third-party weight
licensing/download (a real licensed model package is `#1461`); Registry key
distribution/rotation service; key revocation after registration (takes
effect on re-activation); microphone/codecs; UI; Callweave workflow composition;
cloud LMM transport; training; guest `model_invoke` in v1; Spec 045
candidate semantics.

Browser embedder cross-target report is sequenced after native conformance
under this same governing ID.

## Downstream consumption

Apps invoke only via `exact_model_dependencies` pins, host stage/read APIs,
and Spec 137 `model.execute`. No local host-model fallback and no
provider-specific client API.

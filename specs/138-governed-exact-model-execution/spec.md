# Feature Specification: Governed Exact-Ref Model Execution

**Feature Branch**: `codex/issue-1435-governed-exact-model-execution`
**Created**: 2026-09-16
**Status**: Approved (2026-09-16)
**Canonical governing ID**: `138-governed-exact-model-execution`
**Version**: 0.2.0
**Extends**: `137-host-connector-command-dispatch`,
`044-application-bundle-manifest`, `526-embedded-verified-cache-lifecycle`,
`1259-portable-authority-contracts`, and Registry signed-artifact verification.
**Amendment (2026-09-18, version 0.1.0 -> 0.2.0, approved 2026-09-18)**: Decision 97 /
Spec `140-host-authority-wit-adapters`. Host staging is generalized from
model input/output to bounded host artifacts, including audio produced by
`traverse.audio-input`. Model-named APIs remain valid.

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

- `model_id`, semantic `version`, and package/pair `digest`;
- Registry reference;
- whether offline execution is allowed after provisioning.

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
- license identifier, attribution, and redistribution terms;
- supported target profiles (at least `wasm-cpu` for conformance);
- maximum linear memory, fuel/instruction, input bytes, output bytes, and
  execution time;
- optional quantization / numeric precision metadata;
- provenance / source revision and build reproducibility evidence;
- whether offline execution is allowed after provisioning.

Validation MUST reject missing license, digest, ABI, schema refs, or
resource limits. A model URL alone is never an acceptable identity.
Unknown fields fail closed.

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

## Acceptance scenarios

### Happy paths

1. Exact signed model resolves from Spec 526 active generation and executes
   on `wasm-cpu`.
2. Staged bounded input yields schema-valid output via `output_ref`.
3. Offline execution succeeds with a warm verified cache.
4. Trace identifies model id, version, digest, placement, usage, and
   classification.
5. (Follow-on) Acceleration reports the same public envelope as CPU baseline.

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

## Compatibility and non-goals

Non-goals: production animal-recognition model selection; third-party weight
licensing/download; microphone/codecs; UI; Callweave workflow composition;
cloud LMM transport; training; guest `model_invoke` in v1; Spec 045
candidate semantics.

Browser embedder cross-target report is sequenced after native conformance
under this same governing ID.

## Downstream consumption

Apps invoke only via `exact_model_dependencies` pins, host stage/read APIs,
and Spec 137 `model.execute`. No local host-model fallback and no
provider-specific client API.

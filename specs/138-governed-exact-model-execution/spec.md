# Feature Specification: Governed Exact-Ref Model Execution

**Feature Branch**: `codex/issue-1435-governed-exact-model-execution`
**Created**: 2026-09-16
**Status**: Approved (2026-09-16)
**Canonical governing ID**: `138-governed-exact-model-execution`
**Version**: 0.12.0
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
**Amendment (2026-10-01, version 0.7.0 -> 0.8.0, approved 2026-10-01)**: Decision 107 /
`#1599`. Adds consumer-side model-rights enforcement:
- an app-declared `model_usage` checked against a fixed table, which the
  host may only tighten;
- a structured rights-denial `detail`;
- a host-owned package status map (`deprecated` / `revoked`), re-checked at
  every execute;
- an optional `rights.derivation` in manifest schema `2.1.0`;
- a rights record in execution evidence;
- a shared rights conformance suite.

New reasons: `usage_undeclared`, `rights_policy_denied`, `package_revoked`
and `rights_inconsistent` (FR-035 through FR-042). Breaking for apps:
`model_usage` is required when model pins exist.
**Amendment (2026-10-01, version 0.8.0 -> 0.9.0, approved 2026-10-01)**: Decision 105 /
`#1589`. Adds **two-tier provenance**: FR-024 through FR-027 apply only to
Traverse-published trained packages. Third-party packages MUST instead pass
`traverse-cli model verify` and ship a conformance vector that passes
`model conformance check` on `wasmi` and at least one other engine. The
`traverse-cli model` packaging commands are now governed. Packages built
from a source artifact record it in `rights.derivation` (Decision 107
supersedes Decision 105's free-text `rights.attribution` note). FR-043
through FR-045. Additive; no ABI or schema change.
**Amendment (2026-10-01, version 0.9.0 -> 0.10.0, approved 2026-10-01)**: Decision 108 /
ADR-0079 / `#1580`. Adds **Kotlin/Android** model execution. The framed
model-host protocol moves into the shared safe `traverse-model-host-frame`
crate, which runs behind the Apple C ABI and a new Android JNI shim
(`traverse-android-host`, one audited `modelCall`), on `wasmi` with SIMD. A
host whose native model engine cannot load fails closed with the new
embedder reason `engine_unavailable` (FR-046 through FR-048). Additive.
**Amendment (2026-10-01, version 0.10.0 -> 0.11.0, approved 2026-10-01)**: Decision 103 /
ADR-0080 / `#1567`. Adds **production model-signing key management**:
- a dedicated Ed25519 key held only in the protected `model-signing`
  environment;
- a dispatch-only signing workflow that verifies against the committed
  public keys;
- public keys committed under `keys/model-signing/` and mirrored into an
  opt-in `TRAVERSE_MODEL_SIGNING_KEYS` constant;
- overlap rotation, and revocation by an emergency patch release.

FR-049 through FR-052. Additive; no format change.
**Amendment (2026-10-08, version 0.11.0 -> 0.12.0, approved 2026-10-07)**: Decision 110 /
`#1623` / `#1625`. Adds two project-wide rules for on-device latency:
- **Guest ABI v3 with pristine-snapshot reuse.** The guest exports
  `model_prepare`. The host prepares once under a separate prepare budget,
  snapshots the guest state, and restores that snapshot into a fresh
  instance on every execute. Snapshots live in a host-owned, budgeted,
  in-memory cache. Output is byte-identical to the fresh path.
- **Native accelerator adapters.** Signed per-adapter variants live inside
  the package. Execution policy lists allowed placements in preference
  order, with explicit, traced fallback. Accelerated output meets a declared
  tolerance against wasm-cpu, checked on the device. Each adapter declares a
  ceiling profile. Core ML is the first adapter.

Manifest schema `2.2.0` (additive), connector contract `2.2.0` (additive
`placement_evidence`), new reason `placement_unavailable`. FR-032 is
amended; FR-053 through FR-066 are new.

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

### Model usage (0.8.0, Decision 107)

An application that declares any `exact_model_dependencies` MUST also declare
`model_usage`: `commercial` or `non_commercial`. A missing value fails closed
with `model_incompatible` / `usage_undeclared`. The host MAY require
commercial usage. It can never relax a `commercial` app to `non_commercial`.
The **effective usage** is `commercial` when either the app or the host says
so, and otherwise `non_commercial`.

Every host applies the same fixed table to the signed `rights.commercial_use`.
It runs after the exact pin match, so the pin has already acknowledged the
package's terms.

| signed `commercial_use` | effective `commercial` | effective `non_commercial` |
|---|---|---|
| `allowed` | accept | accept |
| `restricted` | accept only if the pin declares `restricted` | accept only if the pin declares `restricted` |
| `prohibited` | deny | accept |

A denial is `model_incompatible` / `rights_policy_denied`. There is no host
license allowlist and no SPDX compatibility evaluation.

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
- optional `rights.derivation` (schema `2.1.0` only, 0.8.0): `kind`
  (`converted` | `quantized` | `fine_tuned`), `source_digest` (SHA-256 of
  the source artifact), `source_license_id`, `source_commercial_use`, and
  `source_url`. When present, every field is required and non-empty. The
  package `commercial_use` MUST NOT be more permissive than
  `source_commercial_use`, using the order `prohibited < restricted <
  allowed`; a violation is `rights_inconsistent`;
- optional quantization / numeric precision metadata;
- provenance / source revision and build reproducibility evidence;
- whether offline execution is allowed after provisioning;
- schema `2.2.0` only (0.12.0): `max_prepare_fuel`, required when and only
  when `abi_version` is `3` (see "Guest ABI v3"), and optional
  `accelerator_variants` (see "Native accelerator adapters").

Validation MUST reject missing rights, digest, ABI, schema refs, or
resource limits, and any `schema_version` other than `2.0.0`, `2.1.0` or
`2.2.0` (0.8.0; `2.2.0` since 0.12.0). `rights.derivation` in a `2.0.0`
manifest is `manifest_invalid`. `abi_version: 3`, `max_prepare_fuel` or
`accelerator_variants` in a `2.0.0` or `2.1.0` manifest is
`manifest_invalid`. A model URL
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
`crypto_unavailable`, `candidate_unsupported`, `host_limit_exceeded` (0.6.0),
`usage_undeclared`, `rights_policy_denied`, `rights_inconsistent` (all
`model_incompatible`), and `package_revoked` (`model_unavailable`) (0.8.0),
and `engine_unavailable` (`model_unavailable`) (0.10.0) when an embedder's
native model engine cannot load, and `placement_unavailable`
(`model_unavailable`) (0.12.0) when no placement listed by the execution
policy can run the package.

### Rights denial detail (0.8.0, Decision 107)

These failures MUST carry a `detail` object: `rights_incomplete`,
`rights_mismatch`, `rights_policy_denied`, `rights_inconsistent`,
`package_revoked` and `usage_undeclared`. The object has:
- `model_id`, `version` and `digest` (omitted only when no package is known
  yet);
- `field`: the dotted path that failed, for example
  `rights.commercial_use`;
- `expected` and `actual`: strings;
- `effective_usage` (when it was decided).

The shape is identical in every host, so a UI can explain a denial without
re-deriving policy.

### Package status (0.8.0, Decision 107)

The host owns a package status map from package digest to
`{status: deprecated | revoked, reason}`. A package absent from the map is
`active`. Traverse does not distribute the map, and no check uses the
network. The host MAY replace the map at any time.

The status is checked at registration and on every `model.execute`, which
is a map lookup with no cryptography:
- A `revoked` package fails with `model_unavailable` / `package_revoked`.
  A revocation therefore blocks the very next execute.
- A `deprecated` package runs normally. Its status and reason appear in the
  rights record and in the execution evidence.

### Rights record and execution evidence (0.8.0, Decision 107)

A host exposes a **rights record** per registered package, and every
successful `model.execute` result carries the same record as
`model_evidence`. The record contains:
- `model_id`, `version` and `digest`;
- the full signed `rights` (including `derivation`);
- `status` (`active` | `deprecated`, plus `revoked` on a host query; a
  revoked package never executes, so evidence is never `revoked`);
- `status_reason` (when deprecated);
- `effective_usage`.

Tensor bytes and secrets stay redacted (FR-011).

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
  failure codes; detect independently; fail or fall back explicitly. The
  governed rules are under "Native accelerator adapters" (0.12.0).
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
  unchanged. `abi_version` values above 2 are `manifest_invalid` (0.12.0:
  above 3; see "Guest ABI v3").

### Guest ABI v3 (0.12.0, Decision 110)

v3 is v2 plus a separate prepare step, so a host can snapshot the guest
after expensive one-time work (for example an ONNX runner loading and
optimizing its model) and skip that work on later calls.

- `abi_version: 3` requires manifest schema `2.2.0` and `max_prepare_fuel`.
  The guest exports `model_alloc` and `model_execute` exactly as in v2, plus
  `model_prepare() -> i32`. `0` means prepared; any other value is a
  failure.
- A v3 guest's only mutable state is its linear memory and its mutable
  globals. Registration MUST reject a v3 module with `model_incompatible`
  when:
  - it defines a mutable global that it does not export;
  - it imports anything (as for every guest);
  - it uses any table- or segment-mutating instruction (`table.set`,
    `table.grow`, `table.fill`, `table.copy`, `table.init`, `elem.drop`,
    `data.drop`);
  - it does not export exactly one memory;
  - `model_prepare` is missing.
- **Fresh path.** The host instantiates a new `Store` and instance, calls
  `model_prepare` under the prepare budget, then runs the v2 sequence
  (`model_alloc`, input write, `model_execute`).
- **Snapshot path.** See "Prepared-snapshot reuse". Both paths MUST produce
  byte-identical output and byte-identical failures for the same input.
- A trapping or fuel-exhausted `model_prepare`, or a non-zero return, fails
  the call with `execution_failed`.

### Prepared-snapshot reuse (0.12.0, Decision 110)

- **Snapshot.** Right after a successful `model_prepare` on the fresh path,
  and before `model_alloc`, the host MAY record a snapshot. A snapshot holds
  the full linear memory bytes, the memory size in pages, and the value of
  every exported mutable global.
- **Restore.** Every execute that uses a snapshot creates a new `Store` and
  instance from the verified module. It grows memory to the snapshot size,
  overwrites the whole memory with the snapshot bytes, and sets every
  exported mutable global to its snapshot value. It then runs the v2
  sequence. Hosts MUST NOT reuse a live instance or `Store` across calls,
  so per-call isolation is unchanged.
- **Cache.** Snapshots belong to a host-owned in-memory cache:
  - It is keyed by package digest and engine, with at most one snapshot per
    key. Concurrent cold calls MAY each run prepare.
  - A host-configured `max_snapshot_bytes` ceiling (FR-028) bounds the
    total, with a safe default. Over budget, the host evicts snapshots
    (least recently used first) or declines to store one. The affected call
    takes the fresh path, which only costs time.
  - Snapshots are dropped when the package is unregistered, when its status
    changes in the package status map (FR-038), when an execute's digest
    re-check fails (FR-021), and at shutdown.
  - Snapshots are never persisted, shared between hosts, or exposed to
    applications. There is no public warm-up command (FR-003).
- **Prepare budget.**
  - `model_prepare` runs under its own fuel ceiling: manifest
    `max_prepare_fuel` ∩ the host fuel ceiling. It runs in fuel slices like
    execution (FR-029).
  - Its memory is bounded by the same memory ceiling as execution.
  - A call's `max_fuel` (manifest ∩ policy ∩ per-call) covers only the
    v2 sequence, so a call's outcome never depends on whether a snapshot
    existed.
  - Cancellation or a deadline that interrupts `model_prepare` fails the
    triggering call with `cancelled` / `timeout`, and no snapshot is kept.
  - The call's wall-clock deadline (`max_execution_ms` / `timeout_ms`)
    includes prepare time.
- **Evidence.** `resource_usage.fuel_consumed` reports only the v2 sequence.
  Whether a snapshot was used MAY appear in the host's internal trace, but
  MUST NOT appear in the public result, so results stay identical across
  cache states.

## Native accelerator adapters (0.12.0, Decision 110)

An **adapter** runs a package on a native accelerator instead of the
`wasm-cpu` guest, while keeping every public envelope, limit and failure
code of FR-012. These rules apply to every adapter and every host. Core ML
(`coreml`, Apple hosts) is the first adapter. Each new adapter id is added
by amending this list:

| Adapter id | Variant `format` | Hosts |
|---|---|---|
| `coreml` | `mlmodel` (one uncompressed Core ML model file, compiled on the device and cached by variant digest) | Swift (iOS/macOS) |

### Variants inside the package

A manifest (schema `2.2.0`) MAY carry `accelerator_variants`, an array with
at most one entry per adapter id. Each entry has these fields, all required:
- `adapter`: an adapter id from the table above;
- `format`: the adapter's variant format;
- `path`: the variant file's path relative to the package root, under
  `variants/<adapter>/`;
- `digest`: SHA-256 of the variant file bytes;
- `conversion`: `{ tool, tool_version, source_digest }`. It records a
  deterministic conversion from the package's source artifact.
  `source_digest` MUST equal `rights.derivation.source_digest`, so a
  package with variants MUST carry `rights.derivation`. A mismatch is
  `rights_inconsistent`;
- `conformance_vector_path` and `conformance_vector_digest`: the variant's
  conformance vector. The vector's cases are inputs, each with the
  `wasm-cpu` reference output that the signed package produces;
- `tolerance`: `{ max_abs_diff, top_k }`.
  - `max_abs_diff` is a number in `[0, 5e-3]` (Decision 106), compared
    element-wise on `f32` outputs.
  - `top_k` is an integer `≥ 0`. When it is above `0`, the indices of the
    `top_k` largest output values MUST be identical in the same order.
  - Outputs whose dtype is not `f32` MUST match byte-for-byte.
- `peak_memory_bytes`: the declared peak memory for one execution.

The FR-018 manifest signature covers every variant through its digest.
Variants never change the package's pin, identity, rights or status:
- A host fetches and verifies only the variants it may use. A host with no
  matching adapter ignores `accelerator_variants`, but still validates their
  shape.
- A variant file whose bytes do not match its digest fails the call with
  `model_incompatible` / `digest_mismatch`. This is an integrity failure,
  so the host MUST NOT fall back.
- When a variant's `peak_memory_bytes` exceeds the host memory ceiling, the
  host treats that variant as unavailable. The package still registers on
  its other placements.

### Placement selection

The host-owned execution policy named by `policy_ref` carries `placements`:
an ordered, duplicate-free list of placement ids, for example
`["coreml", "wasm-cpu"]`. The default is `["wasm-cpu"]`. Activating a
policy fails closed when it lists an id that is neither `wasm-cpu` nor a
known adapter id.

For each execute, the host takes the first listed placement that meets all
of these:
- the host supports it;
- the package carries it (`wasm-cpu` always, an adapter only through its
  variant);
- it passed its device self-check (below);
- it meets the policy's `require_fuel_metering` flag (below).

Placements not listed are never used. If no listed placement qualifies, the
call fails closed with `model_unavailable` / `placement_unavailable`, before
any input is consumed. The result's `placement` and the trace record the
placement that actually ran.

### Device self-check

The first time a host loads a variant, it runs every case of the variant's
conformance vector through the adapter and compares each output with the
reference under the variant's `tolerance`. The host stores the result in
host-owned state, keyed by variant digest, adapter, OS version, and the
compute units the adapter may use. It runs the check again when any part of
that key changes.

A failed self-check makes the variant unavailable on that device, and
placement selection moves on. The failure appears in the host's trace with
the worst case found. A self-check is not a public call, so its cost is not
charged to any call's ceilings.

### Ceiling profile

Each adapter declares which ceilings it enforces. Every adapter MUST
enforce:
- the call's wall-clock deadline;
- `max_output_bytes`;
- input and output frame dtype and shape checks against the manifest
  schemas;
- the variant's `peak_memory_bytes` against the host memory ceiling.

Cancellation and deadlines MUST be checked before and after the native
prediction. An adapter that cannot interrupt a prediction MUST discard a
result that completes after cancellation (`cancelled`) or after the
deadline (`timeout`).

Fuel is engine-relative (FR-030). An adapter that cannot meter fuel
declares `fuel_metering: not_applicable`. The success result then omits
`resource_usage.fuel_consumed`, and `placement_evidence` records that fuel
was not metered. A policy with `require_fuel_metering: true` excludes every
such adapter.

| Adapter | Fuel | Mid-run interruption | Compute units recorded |
|---|---|---|---|
| `coreml` | `not_applicable` | No: checked before and after; a late result is discarded | the configured `MLComputeUnits` |

### Adapter I/O

An adapter consumes the same staged input frame and produces the same
little-endian output frame, output schema and `output_ref` as the `wasm-cpu`
guest. Converting to and from native tensors is the adapter's job; guests
and applications never see native tensor types.

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
- placement (`wasm-cpu` initially; an adapter id such as `coreml` since
  0.12.0), plus `placement_evidence` (0.12.0): `{ placement, compute_units,
  fuel_metering: metered | not_applicable }`;
- trace ID and redacted evidence, including the rights record
  (`model_evidence`, 0.8.0);
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

### Third-party packages (0.9.0, Decision 105)

A package that is not a Traverse-published trained model (for example a
converted pretrained model, or any team's own guest) does not need
in-repo training provenance. It MUST instead:

- pass `traverse-cli model verify`. That command runs the same registration
  checks every host runs (signature by a trusted key, manifest and rights
  including `rights.derivation`, WASM digest, target, host ceilings) and
  additionally requires zero WASM imports;
- ship a conformance vector (`traverse-cli model conformance generate`)
  whose cases produce byte-identical output on `wasmi` and at least one
  other engine (`wasmtime`), checked by `model conformance check`;
- record any source artifact it was built from in `rights.derivation`.

The packaging workflow is documented in `docs/model-packaging-guide.md`.

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
- **FR-024**: A Traverse-published trained model package MUST vendor its licensed training and
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
- **FR-032**: Hosts MUST support guest ABI v1, v2 and v3 (v3 since 0.12.0),
  selected by the manifest `abi_version`, and MUST reject any other value as
  `manifest_invalid`. `abi_version: 3` is valid only in manifest schema
  `2.2.0`.
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
- **FR-035**: An app with model pins MUST declare `model_usage`; hosts MUST
  fail closed with `usage_undeclared` when it is missing, and MUST let the
  host tighten but never relax it.
- **FR-036**: Hosts MUST apply the fixed usage table at registration and on
  every execute, denying with `rights_policy_denied`.
- **FR-037**: The rights failures listed under "Rights denial detail" MUST
  carry the structured `detail`.
- **FR-038**: Hosts MUST accept a host-owned, replaceable package status map
  and check it at registration and on every execute; `revoked` fails with
  `package_revoked`, and `deprecated` runs but is flagged.
- **FR-039**: Hosts MUST accept manifest schemas `2.0.0` and `2.1.0`,
  validate `rights.derivation`, and reject a derivative more permissive than
  its source with `rights_inconsistent`.
- **FR-040**: Every successful `model.execute` MUST carry the rights record as
  `model_evidence`; hosts MUST expose the same record for a registered
  package.
- **FR-041**: A shared, data-only rights conformance suite
  (`fixtures/models/rights-conformance/`) MUST cover:
  - permissive, non-commercial and restricted packages;
  - missing rights and an unknown `commercial_use`;
  - signature and digest mismatch;
  - revoked and deprecated packages;
  - offline cache-only activation;
  - derivation;
  - evidence.

  Every model-capable embedder MUST pass it with identical codes, reasons,
  details and evidence.
- **FR-042**: Rights enforcement MUST NOT encode application-, model-,
  species-, location- or UI-specific policy.
- **FR-043**: FR-024 through FR-027 apply only to Traverse-published trained
  packages. A third-party package MUST pass `traverse-cli model verify` and
  ship a conformance vector that passes on `wasmi` and at least one other
  engine.
- **FR-044**: `traverse-cli model verify` MUST apply the runtime's own
  registration rules (it MUST NOT re-implement them) plus a zero-import
  check, and MUST exit non-zero with the stable `code` / `reason` /
  `detail` on failure. `traverse-cli model sign` MUST NOT generate or write
  signing keys.
- **FR-045**: `traverse-cli model conformance` MUST run every case through
  the signed register → execute path on `wasmtime` and `wasmi`, and MUST
  fail when the engines' outputs differ or differ from the vector.
- **FR-046**: The Kotlin/Android embedder MUST run exact-ref packages on
  `wasmi` (SIMD) through the shared framed protocol, behind one audited JNI
  method (ADR-0079). It MUST NOT re-implement the verification, rights, or
  policy rules.
- **FR-047**: When the native model engine cannot load, every model call MUST
  fail closed with `model_unavailable` / `engine_unavailable`, with no
  fallback engine.
- **FR-048**: The Kotlin embedder MUST pass the signed vectors byte-for-byte
  (classifier, digits-mlp, digits-onnx) and the shared rights conformance
  suite. Until Android-emulator coverage lands (`#1611`), it runs them on a
  host-JVM build of the same JNI library.
- **FR-049**: The production model-signing private key MUST exist only as
  the `MODEL_SIGNING_KEY_HEX` secret of the protected `model-signing`
  environment. It MUST be used only by the `workflow_dispatch`-only signing
  workflow, and MUST NOT be available to `pull_request` workflows or written
  inside the repository.
- **FR-050**: Production public keys MUST be committed under
  `keys/model-signing/<key_id>.pub`. `TRAVERSE_MODEL_SIGNING_KEYS` in each
  embedder MUST mirror that directory exactly, and trust MUST stay opt-in.
- **FR-051**: The signing workflow MUST verify every signed package against
  the committed public keys before publishing the signing branch.
- **FR-052**: Keys MUST rotate with an overlap window of one minor release
  (yearly or on compromise). A compromised key MUST be revoked by removing it
  from `TRAVERSE_MODEL_SIGNING_KEYS` in an emergency patch release, with a
  security advisory.
- **FR-053**: For ABI v3, registration MUST reject with `model_incompatible`
  a module that:
  - lacks `model_prepare`;
  - defines a non-exported mutable global;
  - uses a table- or segment-mutating instruction;
  - does not export exactly one memory;
  - imports anything.

  A failing `model_prepare` (trap, exhausted fuel, non-zero return) MUST
  fail the call with `execution_failed`.
- **FR-054**: A v3 execute MUST run in a fresh `Store` and instance. It
  either runs `model_prepare` (fresh path) or restores the pristine
  post-prepare snapshot (memory bytes, memory size, exported mutable
  globals). Hosts MUST NOT reuse a live instance or `Store` across calls.
- **FR-055**: Snapshots MUST live in a host-owned in-memory cache keyed by
  package digest and engine. A host-configured `max_snapshot_bytes` ceiling
  with a safe default MUST bound the cache. Over budget, the host evicts a
  snapshot or does not store one, and falls back to the fresh path.
  Snapshots MUST be dropped:
  - on unregister;
  - on a package status change;
  - on a failed digest re-check;
  - at shutdown.

  Snapshots MUST NOT be persisted or exposed to applications.
- **FR-056**: `model_prepare` MUST run under `max_prepare_fuel` ∩ the host
  fuel ceiling, in fuel slices, within the execution memory ceiling. A
  call's `max_fuel` MUST cover only the v2 sequence. Cancellation or a
  deadline during prepare MUST fail the call (`cancelled` / `timeout`) and
  keep no snapshot.
- **FR-057**: For the same package and input, a v3 call's public result
  (status, output bytes, failure code and reason, `resource_usage` except
  `duration_ms`) MUST be identical whether it took the fresh path or the
  snapshot path. A v3 conformance fixture MUST prove this on wasmtime,
  `wasmi`, the browser, the Swift host, and the Kotlin host.
- **FR-058**: Manifest schema `2.2.0` MUST validate `accelerator_variants`:
  - at most one entry per known adapter id;
  - every field present;
  - `path` under `variants/<adapter>/`;
  - `tolerance.max_abs_diff` in `[0, 5e-3]` and `tolerance.top_k ≥ 0`;
  - `conversion.source_digest` equal to `rights.derivation.source_digest`
    (`rights_inconsistent` otherwise).

  Any other violation is `manifest_invalid`.
- **FR-059**: Hosts MUST fetch and verify only the variants they may use. A
  variant whose bytes differ from its manifest digest MUST fail the call
  with `digest_mismatch`, without falling back.
- **FR-060**: Execution policy MUST carry an ordered `placements` list
  (default `["wasm-cpu"]`). Activating a policy that names an unknown id
  MUST fail. Each execute MUST use the first listed placement that the host
  supports, the package carries, has passed its self-check, and meets
  `require_fuel_metering`. If none qualifies, the call MUST fail with
  `model_unavailable` / `placement_unavailable` before consuming the input.
- **FR-061**: The success result and the public trace MUST record the
  placement that ran and `placement_evidence` (`placement`,
  `compute_units`, `fuel_metering`).
- **FR-062**: Before first use on a device, a host MUST check a variant
  against its conformance vector under its declared `tolerance`. The result
  MUST be kept per variant digest, adapter, OS version and compute units,
  and the check MUST run again when any of these change. A failed variant
  MUST be unavailable on that device.
- **FR-063**: Every adapter MUST enforce the deadline, `max_output_bytes`,
  frame dtype and shape, and `peak_memory_bytes` against the host memory
  ceiling. It MUST check cancellation and the deadline before and after a
  prediction it cannot interrupt, and MUST discard a late result as
  `cancelled` / `timeout`.
- **FR-064**: An adapter that cannot meter fuel MUST declare
  `fuel_metering: not_applicable` and omit `fuel_consumed`. A policy with
  `require_fuel_metering: true` MUST exclude it.
- **FR-065**: Adapters MUST consume and produce the same frames, output
  schema and opaque refs as the `wasm-cpu` guest. They MUST NOT
  re-implement verification, rights, status or policy rules: every
  placement shares the host's single registration and execute checks.
- **FR-066**: The adapter id list in "Native accelerator adapters" is
  closed. Adding an adapter (for example for Android or the browser)
  requires a Spec 138 amendment that gives its variant format and ceiling
  profile. Adapter rules MUST NOT encode application-, model- or
  platform-specific behaviour beyond that table.

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
10. (0.12.0) A v3 package's first execute prepares and stores a snapshot.
    Later executes restore it in a fresh instance without calling
    `model_prepare`. Every result is byte-identical to the fresh path on
    every host, including after an eviction.
11. (0.12.0) With policy `["coreml", "wasm-cpu"]`, a package carrying a
    `coreml` variant that has passed its self-check runs on `coreml`.
    Output is within the variant's tolerance of `wasm-cpu`, and
    `placement_evidence` records `coreml`, the compute units and
    `fuel_metering: not_applicable`. On a host without Core ML, the same
    call runs on `wasm-cpu`.

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
    any input is written; `abi_version: 3` is `manifest_invalid` (0.12.0:
    only in schemas `2.0.0` / `2.1.0`; `abi_version: 4` always).
22. (0.8.0) A `commercial` app registering a `prohibited` package →
    `rights_policy_denied` with `detail`; a `non_commercial` app accepts it;
    a host requiring commercial usage denies it even for a `non_commercial`
    app; an app with pins and no `model_usage` → `usage_undeclared`.
23. (0.8.0) A package revoked in the status map → `package_revoked` at
    registration, and at the next execute after a mid-session revocation; a
    deprecated package executes and its evidence shows `status: deprecated`.
24. (0.8.0) A `2.1.0` derivative whose `commercial_use` is more permissive
    than its source → `rights_inconsistent`; `derivation` in `2.0.0` →
    `manifest_invalid`; a valid derivative's `derivation` appears unchanged in
    the rights record and evidence.
25. (0.9.0) A package built with only `traverse-cli model` (sign, pin,
    verify, conformance) from an unsigned manifest and WASM registers on
    native and web hosts. A tampered, untrusted, rights-inconsistent,
    over-ceiling, or importing package fails `model verify` with the same
    reason a host reports.
26. (0.10.0) The Kotlin `ExactModelHost` registers and executes the signed
    vectors byte-identically to native, web, and Swift, passes all 21 rights
    conformance cases, and fails closed with `engine_unavailable` when the
    native library is missing.
27. (0.12.0) A v3 module with a non-exported mutable global, a
    `table.set` / `data.drop`, or no `model_prepare` → `model_incompatible`
    at registration. A trapping, out-of-fuel or non-zero `model_prepare` →
    `execution_failed`, with no snapshot stored. A cancellation during
    prepare → `cancelled`, with no snapshot stored.
28. (0.12.0) A call whose `max_fuel` covers `run` but not prepare plus run
    succeeds on both a cold and a warm cache. A package revocation or a
    status change drops its snapshot.
29. (0.12.0) A tampered variant file → `digest_mismatch`, with no fallback.
    A variant whose `conversion.source_digest` differs from
    `rights.derivation.source_digest` → `rights_inconsistent`. A
    `tolerance.max_abs_diff` above `5e-3` → `manifest_invalid`.
30. (0.12.0) A policy listing only `coreml`, on a host without Core ML or
    after a failed self-check → `placement_unavailable`, and the input is
    not consumed. A policy naming an unknown placement id fails activation.
    A policy with `require_fuel_metering: true` never selects `coreml`.
31. (0.12.0) A cancellation or deadline that lands during a Core ML
    prediction → `cancelled` / `timeout`, and the late result is discarded.

## Compatibility and non-goals

Non-goals: production animal-recognition model selection; third-party weight
licensing/download (a real licensed model package is `#1461`); Registry key
distribution/rotation service; key revocation after registration (takes
effect on re-activation); a Traverse-published or online package status
list (0.8.0: hosts supply the map); SPDX license-compatibility evaluation; microphone/codecs; UI; Callweave workflow composition;
cloud LMM transport; training; guest `model_invoke` in v1; Spec 045
candidate semantics. (0.12.0) Persisting snapshots, or sharing them between
hosts; reusing a live instance; a public warm-up command; per-call
placement chosen by the app; companion accelerator packages; converting
models on the host; accelerator adapters other than those in the adapter
table.

Browser embedder cross-target report is sequenced after native conformance
under this same governing ID.

## Downstream consumption

Apps invoke only via `exact_model_dependencies` pins, host stage/read APIs,
and Spec 137 `model.execute`. No local host-model fallback and no
provider-specific client API.

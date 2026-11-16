# Governed Core ML overhead profile (#1833)

Parent: #1707. Governing spec: `138-governed-exact-model-execution`.

## Purpose

Attribute wall time of one governed Core ML call so a follow-up (#1834) can
cut avoidable copies/rechecks without guessing. Spec 138 checks are unchanged.
#1834's findings and trim are below.

## Harness

Apple-only integration test:

```bash
cargo test --release -p traverse-coreml-adapter profile_governed_coreml_overhead \
  -- --nocapture
```

Write a checked-in evidence JSON (optional):

```bash
TRAVERSE_WRITE_PROFILE_EVIDENCE=1 \
  cargo test --release -p traverse-coreml-adapter profile_governed_coreml_overhead \
  -- --nocapture
```

Fixture: `fixtures/models/digits-onnx-coreml-1.0.0` (same Core ML placement
path as the #1630/#1663 BirdNET evidence family; digits keeps the harness
offline and CI-runnable on macOS runners).

## Stage map

| Stage | What it measures |
|---|---|
| `register` | One-time registration (signature, rights, variant self-check/compile). Excluded from steady execute sum. |
| `stage_input` | Public `ModelIoStore::stage_model_input` frame copy. |
| `execute_total` | Full `model.execute` invoke (digest/rights recheck + Core ML predict + connector glue). |
| `raw_adapter_predict` | `CoreMlAdapter::predict` alone after warm compile. It hashes the variant and vector to key its caches. |
| `adapter_predict_verified` | `CoreMlAdapter::predict_verified`, the entry point `model.execute` uses: no hashing, cached frame contract. |
| `fr021_recheck` | The FR-021 re-hash of the manifest, WASM, variant and vector, timed outside the connector. |
| `digest_rights_and_connector_glue_estimate` | `execute_total - raw_adapter_predict`. |
| `read_output` | Public `ModelIoStore::read_model_output` frame copy. |

## Findings and trim (#1834)

**Always profile a release build.** The #1833 evidence (114 ms per call) was
a debug build. SHA-256 is about 20x slower unoptimized, and the hash
dominates the call. Shipped hosts are release builds; the checked-in JSON now
records `build_profile` and is a release run.

Release build, macOS aarch64, `digits-onnx-coreml-1.0.0` (15.1 MB re-checked):

| Stage | ms | Required? |
|---|---|---|
| `execute_total` | 7.2 | |
| `fr021_recheck` (re-hash manifest, WASM, variant, vector) | 7.1 | Yes: Spec 138 FR-021, every execute |
| `adapter_predict_verified` (Core ML prediction) | 0.03 | Yes |
| policy, rights, status, schema and ceiling checks | under 0.1 | Yes: Spec 138 |
| `stage_input` / `read_output` frame copies | under 0.001 each | Yes: frame protocol |

So a governed call costs about what the FR-021 re-check costs: roughly
0.47 ms per MB of package (about 2.1 GB/s with hardware SHA-256, which the
`sha2` crate uses on every Apple arm64 target). That re-check is a Spec 138
MUST and stays.

What #1834 removed, without changing any Spec 138 check:

- **A second hash of the variant file on every call.** The Core ML adapter
  hashed `variant.bytes` to key its loaded-model cache, right after the
  connector had hashed the same bytes for FR-021. The connector now passes
  the digests it verified (`AcceleratorAdapter::predict_verified`).
- **A full parse of the conformance vector on every call.** The adapter
  parsed the vector JSON to read the frame dtypes. It now parses once per
  vector digest. A vector holds whole input frames as hex, so for a model
  with a 576 KB input each case is over 1 MB of JSON.

Both costs scale with the variant and vector size, so they are small on the
digits fixture (adapter path 0.06 ms to 0.03 ms) and larger on real models.

What remains is required: the FR-021 re-check. Making it cheaper means
changing what Spec 138 requires on every execute, which needs its own
decision.

## Reading results

- If `raw_adapter_predict` dominates `execute_total`, overhead is mostly Core ML / MLMultiArray work.
- If `digest_rights_and_connector_glue_estimate` is large, profile deeper into recheck vs framing before cutting Spec 138 checks.
- Noise of a few ms from OS scheduling is expected (`reconciliation_noise_ms`).

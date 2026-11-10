# Governed Core ML overhead profile (#1833)

Parent: #1707. Governing spec: `138-governed-exact-model-execution`.

## Purpose

Attribute wall time of one governed Core ML call so a follow-up (#1834) can
cut avoidable copies/rechecks without guessing. Spec 138 checks are unchanged.

## Harness

Apple-only integration test:

```bash
cargo test -p traverse-coreml-adapter profile_governed_coreml_overhead \
  -- --nocapture
```

Write a checked-in evidence JSON (optional):

```bash
TRAVERSE_WRITE_PROFILE_EVIDENCE=1 \
  cargo test -p traverse-coreml-adapter profile_governed_coreml_overhead \
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
| `raw_adapter_predict` | `CoreMlAdapter::predict` alone after warm compile. |
| `digest_rights_and_connector_glue_estimate` | `execute_total - raw_adapter_predict`. |
| `read_output` | Public `ModelIoStore::read_model_output` frame copy. |

## Reading results

- If `raw_adapter_predict` dominates `execute_total`, overhead is mostly Core ML / MLMultiArray work.
- If `digest_rights_and_connector_glue_estimate` is large, profile deeper into recheck vs framing before cutting Spec 138 checks.
- Noise of a few ms from OS scheduling is expected (`reconciliation_noise_ms`).

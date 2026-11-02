# Next Release Notes

Unreleased changes since `v0.16.0` will be recorded here.

## Catalog `model.execute@1.0.0` runs in Browser and Node (`#1719`, Decision 122, Spec 1719)

- A guest can now call a connector the host answers asynchronously. It suspends
  at `connector_invoke`, and the host's answer resumes it through the same
  Spec 104 checks. No exported symbol or bridge ABI version changed.
- New in `traverse-embedder-web`: `BundleEmbedder.activateMediatedConnector` and
  `exactModelMediatedConnector`, the first-party `traverse.model-runtime` binding.
  Use `submitAsync` for these targets. Sync `submit` fails them with
  `async_submit_required`.
- The published `model.execute@1.0.0` now runs unchanged over a signed package in
  Node and on native, with one shared vector in
  `fixtures/cross-host/model-execute-v1`. Native also adds `placement_evidence`,
  because only native meters fuel.
- **Fix on every host (Spec 104 1.1.0 FR-008).** The output filter no longer refuses
  a signed `model_evidence` for holding public URLs. Before, a real package failed
  with `connector_unavailable`. The filter still covers everything else.
- `ExactModelExecution` gains `output_schema_ref`.
- Swift, Android and .NET activation APIs aren't shipped yet.

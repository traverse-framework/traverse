# Embedder API Conformance Suite (spec 057)

Platform embedders MUST pass these scenarios for `embedder-api/1.0.0` certification.

## Scenarios

1. **init-shutdown** — `runtime.init` → ready → `runtime.shutdown` → stopped
2. **wasm-capability-submit** — submit single WASM capability → `capability_result` event with expected output shape
3. **compatible-lifecycle** — mock compatible capability: start → stop → kill on shutdown
4. **platform-guard** — compatible capability rejected on wrong platform with deterministic error event
5. **determinism** — same bundled input → identical output JSON twice

## Certification

Embedder implementations record conformance version in app metadata:

```json
{ "traverse_embedder_api": "1.0.0", "conformance_passed": true }
```

Implementation ticket: Traverse **#553** includes this suite.

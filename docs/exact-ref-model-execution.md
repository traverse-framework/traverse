# Exact-ref model execution (Spec 138)

Owner-approved Decisions 91–92. Public invoke remains Spec 137
`dispatch_host_connector_command` → `traverse.model-runtime` /
`model.execute`.

## Surfaces

| Surface | Role |
| --- | --- |
| App `exact_model_dependencies` | Exact pins (`model_id`, `version`, `digest`) |
| `stage_model_input` / `read_model_output` | Host embedder I/O (opaque refs) |
| `stage_artifact` / `read_artifact` | Generic bounded artifacts (Spec 140): opaque, multi-read until drop or shutdown; runtime `ModelIoStore`, web `ModelIoStore`, Swift `ArtifactStagingStore` |
| `model.execute` | Command port; must-match `model_ref`, `policy_ref`, `data_classification` |
| `ExactModelHostConnector` | Native wasm-cpu adapter (`traverse_runtime::exact_model`) |
| Fixture | `fixtures/models/fixture-echo-1.0.0/` |
| `input_from: host_connector_result.<field>` | Spec 139 app-state-machine capability step resolved through the FR-017 runtime-mediated path above (Decision 99) |

## Placement

First conformance profile: `wasm-cpu` (import-denied guest, LE frames).
Browser shares the command/payload contract and
`normalizeModelExecuteEvidence` for cross-target compare; full browser
wasm-cpu guest execution can reuse the same envelopes.

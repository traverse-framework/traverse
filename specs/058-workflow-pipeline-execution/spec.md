# Feature Specification: Workflow Pipeline Execution

**Feature Branch**: `058-workflow-pipeline-execution`
**Created**: 2026-07-06
**Status**: Approved
**Version**: 1.0.0
**Input**: Traverse issue #558, App-References #110/#111, reference-apps multi-capability showcase.

## Purpose

Define **multi-capability workflow pipelines**: a single client submit invokes an ordered chain of capabilities (WASM and/or compatible) and returns **aggregated final output** plus trace events.

This spec extends **007-workflow-registry-traversal** and **052-app-state-machine** for reference-app showcase pipelines without requiring clients to call each capability separately.

## Relationship to Other Specs

| Spec | Relationship |
|------|----------------|
| **007-workflow-registry-traversal** | Pipeline is a governed workflow artifact; deterministic step order. |
| **044-application-bundle-manifest** | App manifest declares pipeline workflow id and step capability refs. |
| **052-app-state-machine** | Optional: pipeline may be invoked via state machine `invoke` or directly by workflow id. |
| **057-embeddable-runtime-host** | `runtime.submit(target_id)` accepts pipeline workflow ids in embedded apps. |
| **033-http-json-api** | Dev sidecar: `POST /execute` accepts pipeline workflow as `capability_id` or dedicated `workflow_id` field (FR-003). |

**No conflict**: single-step execute remains valid; pipeline is additive.

## Reference Pipelines

### traverse-starter.pipeline

| Step | Capability | Mode | Output contribution |
|------|------------|------|---------------------|
| 1 | `traverse-starter.validate` | wasm | `{ valid, issues[] }` |
| 2 | `traverse-starter.process` | wasm | `{ title, tags, noteType, suggestedNextAction, status }` |
| 3 | `traverse-starter.summarize` | wasm | `{ summary, wordCount }` |

Input: `{ "note": string }`

Final output: merge of step outputs under namespaced keys:

```json
{
  "validate": { "valid": true, "issues": [] },
  "process": { "title": "...", "tags": [], "noteType": "...", "suggestedNextAction": "...", "status": "..." },
  "summarize": { "summary": "...", "wordCount": 42 }
}
```

### doc-approval.pipeline

| Step | Capability | Mode |
|------|------------|------|
| 1 | `doc-approval.extract` | wasm |
| 2 | `doc-approval.analyze` | wasm |
| 3 | `doc-approval.recommend` | wasm |

Input: `{ "document": string }`

## Functional Requirements

- **FR-001**: Workflow definitions MUST declare ordered steps with `capability_id` and optional `input_from` (`initial_input`, `previous_output`, or named step output).
- **FR-002**: Runtime MUST execute pipeline steps sequentially unless a later spec adds conditional branching (053).
- **FR-003**: Dev sidecar HTTP API MUST accept pipeline invocation via existing execute endpoint using workflow id as target; response shape unchanged except merged output.
- **FR-004**: Embedded runtime MUST accept the same workflow ids via `runtime.submit` (057).
- **FR-005**: If a step fails, pipeline MUST stop and return deterministic error with failed step id and trace.
- **FR-006**: `traverse-cli app validate` MUST validate pipeline step references resolve to declared components.
- **FR-007**: Pipeline output MUST be runtime-owned; UI renders merged JSON without local field computation.
- **FR-008**: Each pipeline step MUST appear in public trace with step index and capability id.
- **FR-009**: Determinism: same input + same bundled artifacts → identical merged output.

## Definition of Done (implementation)

- [ ] Workflow JSON for `traverse-starter.pipeline` and `doc-approval.pipeline` in Traverse examples
- [ ] Runtime executes full pipeline on dev sidecar and embedded host
- [ ] Integration tests for both pipelines
- [ ] App-References #110/#111 unblocked after WASM agents exist

## Out of Scope

- Parallel pipeline steps
- Dynamic step selection (AI planner)

## Downstream

- Traverse **#554**, **#556**, **#538**, **#555** — WASM agents per step
- reference-apps **#110**, **#111**

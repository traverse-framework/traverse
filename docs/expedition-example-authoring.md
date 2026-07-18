# Expedition Example Authoring Guide

Traverse keeps the canonical expedition example artifacts in three governed locations:

```text
contracts/examples/expedition/
workflows/examples/expedition/
examples/expedition/registry-bundle/
```

Local runtime-owned generated helpers, overlays, and copied execution byproducts belong under:

```text
.traverse/local/
```

See `docs/local-runtime-home.md` for the default layout and ownership boundary.

For agent-package authoring, see `docs/wasm-agent-authoring-guide.md` and the governed example packages under `examples/agents/`.
For WASM microservice authoring, see `docs/wasm-microservice-authoring-guide.md` and the package template under `examples/templates/`.

## Artifact Categories

Atomic capability contracts:

- `contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json`
- `contracts/examples/expedition/capabilities/interpret-expedition-intent/contract.json`
- `contracts/examples/expedition/capabilities/assess-conditions-summary/contract.json`
- `contracts/examples/expedition/capabilities/validate-team-readiness/contract.json`
- `contracts/examples/expedition/capabilities/assemble-expedition-plan/contract.json`

Workflow-backed composed capability contract:

- `contracts/examples/expedition/capabilities/plan-expedition/contract.json`

Event contracts:

- `contracts/examples/expedition/events/expedition-objective-captured/contract.json`
- `contracts/examples/expedition/events/expedition-intent-interpreted/contract.json`
- `contracts/examples/expedition/events/conditions-summary-assessed/contract.json`
- `contracts/examples/expedition/events/team-readiness-validated/contract.json`
- `contracts/examples/expedition/events/expedition-plan-assembled/contract.json`

Workflow artifact:

- `workflows/examples/expedition/plan-expedition/workflow.json`

Registry bundle manifest:

- `examples/expedition/registry-bundle/manifest.json`

## Authoring Rules

- Keep ids and versions aligned with the approved expedition specs.
- Do not invent alternate names for the canonical expedition capabilities, events, or workflow.
- Treat the contracts and workflow artifacts under `contracts/examples/` and `workflows/examples/` as the source of truth.
- Treat the registry bundle manifest as a projection over those governed artifacts, not a replacement for them.

## Validation Commands

Artifact smoke validation:

```bash
bash scripts/ci/expedition_artifact_smoke.sh
```

Registry bundle inspection:

```bash
cargo run -p traverse-cli-rs -- bundle inspect examples/expedition/registry-bundle/manifest.json
```

Registry bundle registration:

```bash
cargo run -p traverse-cli-rs -- bundle register examples/expedition/registry-bundle/manifest.json
```

Expedition execution:

```bash
cargo run -p traverse-cli-rs -- expedition execute examples/expedition/runtime-requests/plan-expedition.json
```

Expedition execution with persisted trace:

```bash
tmpdir="$(mktemp -d)"
cargo run -p traverse-cli-rs -- expedition execute examples/expedition/runtime-requests/plan-expedition.json --trace-out "$tmpdir/plan-expedition-trace.json"
```

Trace inspection:

```bash
cargo run -p traverse-cli-rs -- trace inspect "$tmpdir/plan-expedition-trace.json"
```
Event contract inspection:

```bash
cargo run -p traverse-cli-rs -- event inspect contracts/examples/expedition/events/expedition-objective-captured/contract.json
```

Workflow artifact inspection:

```bash
cargo run -p traverse-cli-rs -- workflow inspect workflows/examples/expedition/plan-expedition/workflow.json
```

Repository checks:

```bash
bash scripts/ci/repository_checks.sh
```

Golden path proof:

```bash
bash scripts/ci/expedition_golden_path.sh
```

## What Good Output Looks Like

The bundle inspection output must include:

- `expedition.planning.capture-expedition-objective`
- `expedition.planning.interpret-expedition-intent`
- `expedition.planning.assess-conditions-summary`
- `expedition.planning.validate-team-readiness`
- `expedition.planning.assemble-expedition-plan`
- `expedition.planning.plan-expedition`

The bundle registration output must include:

- `registered_capabilities: 6`
- `registered_events: 5`
- `registered_workflows: 1`
- `expedition.planning.plan-expedition@1.0.0 (workflow)`

And the workflow section must include:

- `expedition.planning.plan-expedition@1.0.0`
- `registered_workflows: 1`
- `expedition.planning.plan-expedition@1.0.0 (workflow)`

The expedition execution output must include:

- `capability_id: expedition.planning.plan-expedition`
- `status: completed`
- `recommended_route_style: conservative-alpine-push`
- `trace_ref: trace_exec_expedition-plan-request-001`

The trace inspection output must include:

- `trace_id: trace_exec_expedition-plan-request-001`
- `result_status: completed`
- `selected_capability_id: expedition.planning.plan-expedition`

The golden path validation must prove all of these in one run:

- bundle registration succeeds for the canonical manifest
- execution succeeds for the canonical runtime request
- trace inspection succeeds for the generated runtime trace
- a missing required bundle artifact fails deterministically

The event inspection output must include:

- `id: expedition.planning.expedition-objective-captured`
- `event_type: domain`
- `publisher_ids:`

The workflow inspection output must include:

- `id: expedition.planning.plan-expedition`
- `start_node: capture_objective`
- `node_capabilities:`

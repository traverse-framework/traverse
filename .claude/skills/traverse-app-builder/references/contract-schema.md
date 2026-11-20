# Contract, workflow, manifest, and request shapes

All shapes below are verified two ways: against the `CapabilityContract` struct in `crates/traverse-contracts/src/lib.rs` (so field names and required-ness are exact, not guessed), and against a capability built from scratch and run through `traverse-cli agent inspect` to a clean pass. Where the checked-in `scripts/scaffold/new-capability.sh` disagrees with this file, this file is correct and the scaffold script is stale — it predates the current struct and will produce a contract that fails to deserialize.

## Capability contract (`contract.json`)

```json
{
  "kind": "capability_contract",
  "schema_version": "1.0.0",
  "id": "namespace.name",
  "namespace": "namespace",
  "name": "name",
  "version": "1.0.0",
  "lifecycle": "active",
  "owner": { "team": "your-team", "contact": "you@example.com" },
  "summary": "One sentence: what this capability does.",
  "description": "A longer description of the capability's purpose and behavior.",
  "inputs": {
    "schema": {
      "type": "object",
      "required": ["your_field"],
      "properties": { "your_field": { "type": "string" } },
      "additionalProperties": false
    }
  },
  "outputs": {
    "schema": {
      "type": "object",
      "required": ["your_output_field"],
      "properties": { "your_output_field": { "type": "string" } },
      "additionalProperties": false
    }
  },
  "preconditions": [
    { "id": "your-field-provided", "description": "The caller provides a valid your_field." }
  ],
  "postconditions": [
    { "id": "output-produced", "description": "The capability returns your_output_field." }
  ],
  "side_effects": [
    { "kind": "memory_only", "description": "Describe what the capability actually does to state, if anything." }
  ],
  "emits": [],
  "consumes": [],
  "permissions": [
    { "id": "namespace.name" }
  ],
  "execution": {
    "binary_format": "wasm",
    "entrypoint": { "kind": "wasi-command", "command": "run" },
    "preferred_targets": ["local"],
    "constraints": {
      "host_api_access": "none",
      "network_access": "forbidden",
      "filesystem_access": "none"
    }
  },
  "policies": [],
  "dependencies": [],
  "provenance": {
    "source": "greenfield",
    "author": "your-name",
    "created_at": "2026-01-01T00:00:00Z",
    "spec_ref": null,
    "adr_refs": [],
    "exception_refs": []
  },
  "evidence": [],
  "service_type": "stateless",
  "permitted_targets": ["local"],
  "artifact_type": "native"
}
```

**Fields every field is required unless noted.** `emits`, `consumes`, `permissions`, `policies`, `dependencies`, `evidence` all need to be present as arrays even when empty — the deserializer doesn't default them. `service_type` and `permitted_targets` DO have real defaults if you omit them (`service_type` → `"stateless"`, `permitted_targets` → all six targets) — but don't rely on the `permitted_targets` default. It defaults to declaring `local`, `browser`, `edge`, `cloud`, `worker`, and `device` all permitted, which is a governance-level "allowed to run here eventually" declaration, not a claim that the runtime can execute there today. Set it explicitly to what you actually mean — almost always just `["local"]` for a new capability — and cross-check against `references/platforms.md` before claiming more.

**Enum values, verified against the struct:**
| Field | Valid values |
|---|---|
| `lifecycle` | `draft`, `active`, `deprecated`, `retired`, `archived` — only `active` and `deprecated` are runtime-eligible |
| `service_type` | `stateless` (default), `subscribable` (needs `event_trigger`), `stateful` (can't run in Browser) |
| `execution.binary_format` | `wasm` (only value defined) |
| `execution.entrypoint.kind` | `wasi-command` (only value defined) |
| `execution.preferred_targets[]` / `permitted_targets[]` | `local`, `browser`, `edge`, `cloud`, `worker`, `device` |
| `execution.constraints.host_api_access` | `none`, `exception_required` |
| `execution.constraints.network_access` | `forbidden`, `required` |
| `execution.constraints.filesystem_access` | `none`, `sandbox_only` |
| `side_effects[].kind` | `none`, `memory_only`, `event_emission`, `external_call`, `state_change` |
| `provenance.source` | check `ProvenanceSource` enum in the struct if `greenfield` doesn't fit |

## Workflow (`workflow.json`)

A package can't register without at least one approved workflow reference — an empty `workflow_refs: []` in the manifest is rejected outright. The simplest valid workflow is a single node calling your one capability:

```json
{
  "kind": "workflow_definition",
  "schema_version": "1.0.0",
  "id": "namespace.name",
  "name": "name",
  "version": "1.0.0",
  "lifecycle": "active",
  "owner": { "team": "your-team", "contact": "you@example.com" },
  "summary": "Run the namespace.name capability as one governed workflow.",
  "inputs": { "schema": { "...": "same shape as the capability's inputs" } },
  "outputs": { "schema": { "...": "same shape as the capability's outputs" } },
  "nodes": [
    {
      "node_id": "run",
      "capability_id": "namespace.name",
      "capability_version": "1.0.0",
      "input": { "from_workflow_input": ["your_field"] },
      "output": { "to_workflow_state": ["your_output_field"] }
    }
  ],
  "edges": [],
  "start_node": "run",
  "terminal_nodes": ["run"],
  "tags": [],
  "governing_spec": "007-workflow-registry-traversal"
}
```

## Agent package manifest (`manifest.json`)

This is what `traverse-cli agent inspect`/`agent execute` actually reads. It's a lighter-weight package format than the full application-bundle manifest (see SKILL.md's "Actually running it" section for why that distinction matters).

```json
{
  "kind": "agent_package",
  "schema_version": "1.0.0",
  "package_id": "namespace.name-agent",
  "version": "1.0.0",
  "summary": "One sentence.",
  "capability_ref": {
    "id": "namespace.name",
    "version": "1.0.0",
    "contract_path": "relative/path/to/contract.json"
  },
  "workflow_refs": [
    { "workflow_id": "namespace.name", "workflow_version": "1.0.0" }
  ],
  "source": { "path": "./src/agent.rs", "language": "rust", "entry": "run" },
  "binary": {
    "path": "./artifacts/your-agent.wasm",
    "format": "wasm",
    "expected_digest": "fnv1a64:0000000000000000",
    "abi_version": "1.0.0"
  },
  "constraints": {
    "host_api_access": "none",
    "network_access": "forbidden",
    "filesystem_access": "none"
  },
  "model_dependencies": []
}
```

The digest is **FNV-1a-64**, formatted `fnv1a64:<16 lowercase hex chars>` — not SHA-256, even though a couple of docs/scripts in the repo (like the stale scaffold script) compute SHA-256. Don't hand-compute it: put a placeholder in, run `agent inspect`, and the tool's own mismatch error tells you the real value to paste in.

## Composing multiple atomic capabilities into one workflow

A workflow isn't limited to one node. `workflows/examples/expedition/plan-expedition/workflow.json` in the real repo chains five separate, narrowly-scoped capabilities (capture → interpret → assess → validate → assemble) into one governed traversal:

```json
{
  "nodes": [
    { "node_id": "capture_objective", "capability_id": "expedition.planning.capture-expedition-objective", "capability_version": "1.0.0",
      "input": { "from_workflow_input": ["destination", "target_window", "preferences", "notes"] },
      "output": { "to_workflow_state": ["objective"] } },
    { "node_id": "interpret_intent", "capability_id": "expedition.planning.interpret-expedition-intent", "capability_version": "1.0.0",
      "input": { "from_workflow_input": ["objective", "planning_intent"] },
      "output": { "to_workflow_state": ["interpreted_intent"] } }
  ],
  "edges": [
    { "edge_id": "capture_to_interpret", "from": "capture_objective", "to": "interpret_intent", "trigger": "direct" }
  ],
  "start_node": "capture_objective",
  "terminal_nodes": ["assemble_plan"]
}
```

**A naming gotcha worth knowing up front:** despite its name, a node's `from_workflow_input` does not only pull from the workflow's top-level input schema — it pulls from the accumulated workflow *state* bag, which includes both the original inputs and every prior node's `to_workflow_state` output. In the example above, `interpret_intent` reads `objective` via `from_workflow_input` even though `objective` isn't one of the workflow's declared top-level inputs — it only exists because `capture_objective` wrote it a step earlier. Don't be misled by the field name into thinking each node is limited to the workflow's original request payload.

This is also the real design argument for keeping each capability atomic (one verb, one responsibility) rather than folding multiple concerns into a single capability: a workflow only composes cleanly if each node's contract is narrow enough to slot into a chain like this. If you're about to write a single capability that captures, interprets, *and* validates all in one binary, consider whether splitting it into workflow nodes instead would make each piece independently reusable and testable — the same way `interpret-expedition-intent` and `validate-team-readiness` can each be discovered and reused on their own (see `references/registry.md`), whereas a monolithic capability can't be.

## Runtime request (`runtime-request.json`)

```json
{
  "kind": "runtime_request",
  "schema_version": "1.0.0",
  "request_id": "namespace-name-test-001",
  "intent": { "capability_id": "namespace.name", "capability_version": "1.0.0" },
  "input": { "your_field": "example value" },
  "lookup": { "scope": "public_only", "allow_ambiguity": false },
  "context": { "requested_target": "local", "caller": "manual-test" },
  "governing_spec": "006-runtime-request-execution"
}
```

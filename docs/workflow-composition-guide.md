# Workflow Composition Guide

This guide shows how to chain two capabilities into a governed workflow using the Traverse registry and deterministic traversal model.

After completing it you will be able to:

- write two capability contracts whose input and output schemas connect
- write a workflow contract linking them with a direct edge
- register both capabilities and the workflow through the bundle CLI
- invoke the workflow and inspect trace evidence showing both steps executed

**Governing specs**: [007-workflow-registry-traversal](../specs/007-workflow-registry-traversal/spec.md), [018-event-driven-composition](../specs/018-event-driven-composition/spec.md)

---

## Prerequisites

- Rust 1.94 or later
- the repository checked out locally
- the workspace builds cleanly:

```bash
bash scripts/validate-setup.sh
cargo build
```

If you are new to Traverse, complete [docs/getting-started.md](getting-started.md) first. That guide covers how a single capability contract is structured before you compose two together.

---

## What Is a Workflow in Traverse

A workflow is an approved, machine-readable artifact that declares:

- **nodes** — each node maps to one registered capability version
- **edges** — each edge declares how execution advances from one node to the next; edges are either `direct` (sequential) or `event` (emitted-event-triggered)
- **start\_node** and **terminal\_nodes** — explicit entry and exit points
- **governing\_spec** — the spec that governs its registration and traversal rules

The runtime traverses one workflow definition in deterministic order. It does not choose paths or apply heuristics. Every transition must be declared in the workflow file before execution starts.

---

## Step 1: Define Two Capabilities

We will compose two capabilities:

| Role | Capability id |
|---|---|
| Capability A | `content.notes.draft-note` |
| Capability B | `content.notes.tag-note` |

Capability A takes free text and produces a draft id. Capability B takes that draft id and a list of tags and produces a tagged note id. The output of A feeds the input of B through the workflow state.

### Capability A contract

Place this file at:

```
contracts/examples/notes/capabilities/draft-note/contract.json
```

```json
{
  "kind": "capability_contract",
  "schema_version": "1.0.0",
  "id": "content.notes.draft-note",
  "namespace": "content.notes",
  "name": "draft-note",
  "version": "1.0.0",
  "lifecycle": "active",
  "owner": {
    "team": "notes",
    "contact": "notes@example.com"
  },
  "summary": "Create a note draft from free text.",
  "description": "Accepts raw note text and produces a stable draft id for downstream processing.",
  "inputs": {
    "schema": {
      "type": "object",
      "required": ["note_text"],
      "properties": {
        "note_text": { "type": "string" }
      },
      "additionalProperties": false
    }
  },
  "outputs": {
    "schema": {
      "type": "object",
      "required": ["draft_id"],
      "properties": {
        "draft_id": { "type": "string" }
      },
      "additionalProperties": false
    }
  },
  "preconditions": [
    { "id": "note_text_non_empty", "description": "note_text must not be empty" }
  ],
  "postconditions": [
    { "id": "draft_id_produced", "description": "draft_id must be present in outputs" }
  ],
  "side_effects": [
    { "kind": "memory_only", "description": "draft stored in memory only" }
  ],
  "emits": [],
  "consumes": [],
  "permissions": [],
  "execution": {
    "binary_format": "wasm",
    "entrypoint": { "kind": "wasi_command", "command": "run" },
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
    "created_at": "2026-04-17T00:00:00Z",
    "spec_ref": "007-workflow-registry-traversal",
    "adr_refs": [],
    "exception_refs": []
  },
  "evidence": [],
  "service_type": "stateless",
  "permitted_targets": ["local", "cloud", "edge", "device"],
  "event_trigger": null
}
```

**What to notice**: `emits` is empty because this capability advances the workflow through a `direct` edge. For event-triggered advancement you would declare an emitted event here and reference it in the workflow edge — see [Step 2: event edge variant](#event-edge-variant) below.

### Capability B contract

Place this file at:

```
contracts/examples/notes/capabilities/tag-note/contract.json
```

```json
{
  "kind": "capability_contract",
  "schema_version": "1.0.0",
  "id": "content.notes.tag-note",
  "namespace": "content.notes",
  "name": "tag-note",
  "version": "1.0.0",
  "lifecycle": "active",
  "owner": {
    "team": "notes",
    "contact": "notes@example.com"
  },
  "summary": "Attach tags to an existing note draft.",
  "description": "Accepts a draft id and a list of tags and produces a tagged note id.",
  "inputs": {
    "schema": {
      "type": "object",
      "required": ["draft_id", "tags"],
      "properties": {
        "draft_id": { "type": "string" },
        "tags": {
          "type": "array",
          "items": { "type": "string" }
        }
      },
      "additionalProperties": false
    }
  },
  "outputs": {
    "schema": {
      "type": "object",
      "required": ["note_id"],
      "properties": {
        "note_id": { "type": "string" }
      },
      "additionalProperties": false
    }
  },
  "preconditions": [
    { "id": "draft_exists", "description": "draft_id must reference a known draft" }
  ],
  "postconditions": [
    { "id": "note_id_produced", "description": "note_id must be present in outputs" }
  ],
  "side_effects": [
    { "kind": "memory_only", "description": "tags stored in memory only" }
  ],
  "emits": [],
  "consumes": [],
  "permissions": [],
  "execution": {
    "binary_format": "wasm",
    "entrypoint": { "kind": "wasi_command", "command": "run" },
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
    "created_at": "2026-04-17T00:00:00Z",
    "spec_ref": "007-workflow-registry-traversal",
    "adr_refs": [],
    "exception_refs": []
  },
  "evidence": [],
  "service_type": "stateless",
  "permitted_targets": ["local", "cloud", "edge", "device"],
  "event_trigger": null
}
```

**What to notice**: `draft_id` appears in Capability A outputs and also in Capability B inputs. The workflow state carries it between nodes using the `to_workflow_state` / `from_workflow_input` mapping in the workflow contract.

---

## Step 2: Write the Workflow Contract

Place this file at:

```
workflows/examples/notes/draft-and-tag-note/workflow.json
```

```json
{
  "kind": "workflow_definition",
  "schema_version": "1.0.0",
  "id": "content.notes.draft-and-tag-note",
  "name": "draft-and-tag-note",
  "version": "1.0.0",
  "lifecycle": "active",
  "owner": {
    "team": "notes",
    "contact": "notes@example.com"
  },
  "summary": "Draft a note then tag it in one deterministic workflow.",
  "inputs": {
    "schema": {
      "type": "object",
      "required": ["note_text", "tags"],
      "properties": {
        "note_text": { "type": "string" },
        "tags": {
          "type": "array",
          "items": { "type": "string" }
        }
      },
      "additionalProperties": false
    }
  },
  "outputs": {
    "schema": {
      "type": "object",
      "required": ["note_id"],
      "properties": {
        "note_id": { "type": "string" }
      },
      "additionalProperties": false
    }
  },
  "nodes": [
    {
      "node_id": "draft_note",
      "capability_id": "content.notes.draft-note",
      "capability_version": "1.0.0",
      "input": {
        "from_workflow_input": ["note_text"]
      },
      "output": {
        "to_workflow_state": ["draft_id"]
      }
    },
    {
      "node_id": "tag_note",
      "capability_id": "content.notes.tag-note",
      "capability_version": "1.0.0",
      "input": {
        "from_workflow_input": ["draft_id", "tags"]
      },
      "output": {
        "to_workflow_state": ["note_id"]
      }
    }
  ],
  "edges": [
    {
      "edge_id": "draft_to_tag",
      "from": "draft_note",
      "to": "tag_note",
      "trigger": "direct"
    }
  ],
  "start_node": "draft_note",
  "terminal_nodes": ["tag_note"],
  "tags": ["notes", "composition", "example"],
  "governing_spec": "007-workflow-registry-traversal"
}
```

### Reading the workflow contract

| Field | Meaning |
|---|---|
| `nodes[0].input.from_workflow_input` | the workflow runtime takes `note_text` from the top-level workflow input and passes it to node `draft_note` |
| `nodes[0].output.to_workflow_state` | when `draft_note` completes, `draft_id` is written to workflow state |
| `nodes[1].input.from_workflow_input` | `draft_id` comes from workflow state (put there by node 0) and `tags` comes from the original workflow input |
| `edges[0].trigger` | `direct` means the runtime advances immediately when `draft_note` finishes — no event emission required |
| `start_node` | traversal always begins at `draft_note` |
| `terminal_nodes` | traversal ends when `tag_note` completes |
| `governing_spec` | must equal `007-workflow-registry-traversal` — the registry validator enforces this exact string |

### Event edge variant

If Capability A emits a governed event and you want Capability B to wait for it, change the edge to:

```json
{
  "edge_id": "draft_to_tag",
  "from": "draft_note",
  "to": "tag_note",
  "trigger": "event",
  "event": {
    "event_id": "content.notes.note-drafted",
    "version": "1.0.0"
  }
}
```

For an event edge to pass validation, `content.notes.note-drafted@1.0.0` must:

1. have a registered event contract in the registry
2. be declared in Capability A's `emits` array

Event-driven progression semantics are governed by [018-event-driven-composition](../specs/018-event-driven-composition/spec.md).

---

## Step 3: Register Both Capabilities and the Workflow

Traverse registers capabilities and workflows through a registry bundle. Create a bundle manifest at:

```
examples/notes/registry-bundle/manifest.json
```

```json
{
  "kind": "registry_bundle",
  "schema_version": "1.0.0",
  "id": "content.notes.example-bundle",
  "version": "1.0.0",
  "capabilities": [
    {
      "contract_path": "contracts/examples/notes/capabilities/draft-note/contract.json",
      "artifact_ref": "content.notes.draft-note@1.0.0"
    },
    {
      "contract_path": "contracts/examples/notes/capabilities/tag-note/contract.json",
      "artifact_ref": "content.notes.tag-note@1.0.0"
    }
  ],
  "workflows": [
    {
      "workflow_path": "workflows/examples/notes/draft-and-tag-note/workflow.json"
    }
  ]
}
```

Inspect the bundle before registering:

```bash
cargo run -p traverse-cli-rs -- bundle inspect \
  examples/notes/registry-bundle/manifest.json
```

Expected output includes:

- `content.notes.draft-note@1.0.0`
- `content.notes.tag-note@1.0.0`
- `content.notes.draft-and-tag-note@1.0.0 (workflow)`

Register the bundle:

```bash
cargo run -p traverse-cli-rs -- bundle register \
  examples/notes/registry-bundle/manifest.json
```

Expected output:

```
registered_capabilities: 2
registered_workflows: 1
content.notes.draft-and-tag-note@1.0.0 (workflow)
```

The registry validates both capability contracts before accepting the workflow. If either capability is missing or the workflow references an unregistered version the command fails with structured validation output.

---

## Step 4: Invoke the Workflow

Inspect the registered workflow:

```bash
cargo run -p traverse-cli-rs -- workflow inspect \
  workflows/examples/notes/draft-and-tag-note/workflow.json
```

Expected output includes:

- `id: content.notes.draft-and-tag-note`
- `start_node: draft_note`
- the ordered node list

Create a runtime request file at `examples/notes/runtime-requests/draft-and-tag-note.json`:

```json
{
  "capability_id": "content.notes.draft-and-tag-note",
  "version": "1.0.0",
  "input": {
    "note_text": "Traverse makes capability composition explicit and governed.",
    "tags": ["traverse", "governance", "example"]
  }
}
```

Execute it:

```bash
cargo run -p traverse-cli-rs -- execute \
  examples/notes/runtime-requests/draft-and-tag-note.json
```

Expected output:

```
capability_id: content.notes.draft-and-tag-note
status: completed
note_id: <generated-id>
```

---

## Step 5: Inspect the Trace to Verify Both Steps Executed

Persist the trace to a file:

```bash
tmpdir="$(mktemp -d)"
cargo run -p traverse-cli-rs -- execute \
  examples/notes/runtime-requests/draft-and-tag-note.json \
  --trace-out "$tmpdir/draft-and-tag-note-trace.json"
```

Inspect the trace:

```bash
cargo run -p traverse-cli-rs -- trace inspect \
  "$tmpdir/draft-and-tag-note-trace.json"
```

What to look for in the trace output:

- `result_status: completed`
- `visited_nodes` containing both `draft_note` and `tag_note` in that order
- `traversed_edges` containing `draft_to_tag`
- no skipped or missing nodes

If a node is missing from `visited_nodes` the workflow did not complete the full chain. Re-check the edge declarations and the `from_workflow_input` mappings.

---

## Common Mistakes and Troubleshooting

### Workflow registration fails with `MissingReference`

The workflow references a capability that is not registered yet. Register the bundle before inspecting the workflow, or check that the `capability_id` and `capability_version` in the workflow nodes exactly match the `id` and `version` in the capability contracts.

### Workflow registration fails with `InvalidLiteral` on `governing_spec`

The `governing_spec` field in the workflow file must equal exactly:

```
007-workflow-registry-traversal
```

Any other value is rejected.

### Edge rejected with `InvalidEventEdge`

A `direct` edge must not include an `event` or `predicate` field. An `event` edge must include exactly one `event` reference and that event must be declared in the source capability's `emits` array.

### Workflow registration fails with `DeterministicCycleNotAllowed`

In v0.1, cycles in the workflow graph are not permitted. Each node may only appear in one path from `start_node` to a `terminal_node`. If you need conditional branching, use predicates on event edges rather than loops.

### `ImmutableVersionConflict` when re-registering

Once a `(scope, id, version)` triple is registered with a given content digest, it cannot be changed. If you need to update the workflow, bump the `version` field to the next semantic version.

### Trace shows only one node

If only the first node appears in the visited trace, the edge from that node was not followed. Check that:

- the `edge_id` from field matches the `node_id` of the first node
- the `to` field matches the `node_id` of the second node
- the `trigger` field is `direct` (or that the required event was emitted, for event edges)

---

## What You Learned

After this guide you should be able to answer:

- how two capability contracts are connected through `inputs.schema` / `outputs.schema` field overlap
- how `from_workflow_input` and `to_workflow_state` carry values between nodes
- what a `direct` edge means and when to use an `event` edge instead
- how to register a multi-capability bundle and verify both nodes appear in the trace

---

## Next Steps

- [docs/expedition-example-authoring.md](expedition-example-authoring.md) — a five-capability workflow with the full governed expedition domain
- [docs/wasm-agent-authoring-guide.md](wasm-agent-authoring-guide.md) — package a capability as a compiled WASM agent
- [docs/wasm-microservice-authoring-guide.md](wasm-microservice-authoring-guide.md) — package a capability as a WASM microservice
- [specs/007-workflow-registry-traversal/spec.md](../specs/007-workflow-registry-traversal/spec.md) — full workflow registry spec
- [specs/018-event-driven-composition/spec.md](../specs/018-event-driven-composition/spec.md) — event-driven edge semantics

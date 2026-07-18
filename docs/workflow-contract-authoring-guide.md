# Workflow Contract Authoring Guide

This guide shows how to author a valid workflow contract from scratch, register it, and validate it locally.

Use the checked-in examples as living references:

- [`workflows/examples/expedition/plan-expedition/workflow.json`](../workflows/examples/expedition/plan-expedition/workflow.json)
- [`workflows/examples/hello-world/say-hello/workflow.json`](../workflows/examples/hello-world/say-hello/workflow.json)
- [`specs/007-workflow-registry-traversal/spec.md`](../specs/007-workflow-registry-traversal/spec.md)
- [`docs/workflow-composition-guide.md`](workflow-composition-guide.md)

---

## Node/Edge Model

A workflow in Traverse is a directed graph of capability invocations connected by typed edges.

```
workflow input
      |
      v
 [node: capture]  ──direct──>  [node: enrich]  ──event──>  [node: assemble]
      |                                                           |
  to_workflow_state                                          to_workflow_state
      |                                                           |
      v                                                           v
 workflow state                                           workflow output
```

Key concepts:

- **node** — maps to one registered capability version. Each node reads from workflow input or workflow state and writes its output back to workflow state.
- **edge** — connects two nodes. Either `direct` (immediate, sequential) or `event` (triggered by a governed event emission).
- **start\_node** — exactly one. The runtime always begins traversal here.
- **terminal\_nodes** — one or more. Traversal ends when one of these nodes completes.
- **workflow state** — the accumulator that carries values produced by upstream nodes to downstream nodes.

The runtime is deterministic: it does not choose paths or apply heuristics. Every transition must be declared before execution starts.

---

## Minimal Two-Node Annotated Workflow

Place your workflow at:

```
workflows/<domain>/<workflow-name>/workflow.json
```

```json
{
  "kind": "workflow_definition",          // must be exactly "workflow_definition"
  "schema_version": "1.0.0",             // must be "1.0.0" in v0.1
  "id": "acme.orders.draft-and-confirm", // must equal namespace.name
  "name": "draft-and-confirm",           // lowercase kebab-case
  "version": "1.0.0",                    // semver MAJOR.MINOR.PATCH
  "lifecycle": "active",                 // use "draft" during authoring
  "owner": {
    "team": "orders-team",
    "contact": "orders@example.com"
  },
  "summary": "Draft an order and confirm it in a two-step deterministic workflow.",
  "inputs": {
    "schema": {
      "type": "object",
      "required": ["cart_id", "customer_id"],
      "properties": {
        "cart_id":     { "type": "string" },
        "customer_id": { "type": "string" }
      }
    }
  },
  "outputs": {
    "schema": {
      "type": "object",
      "required": ["order_id", "confirmation_code"],
      "properties": {
        "order_id":          { "type": "string" },
        "confirmation_code": { "type": "string" }
      }
    }
  },
  "nodes": [
    {
      "node_id": "draft_order",                         // unique within this workflow
      "capability_id": "acme.orders.draft-order",       // must match a registered capability id
      "capability_version": "1.0.0",                    // must match registered version
      "input": {
        "from_workflow_input": ["cart_id", "customer_id"]  // fields taken from workflow input or state
      },
      "output": {
        "to_workflow_state": ["order_id"]               // fields written to workflow state
      }
    },
    {
      "node_id": "confirm_order",
      "capability_id": "acme.orders.confirm-order",
      "capability_version": "1.0.0",
      "input": {
        "from_workflow_input": ["order_id"]             // "order_id" was written by draft_order above
      },
      "output": {
        "to_workflow_state": ["order_id", "confirmation_code"]
      }
    }
  ],
  "edges": [
    {
      "edge_id": "draft_to_confirm",   // unique within this workflow
      "from": "draft_order",           // node_id of the source node
      "to": "confirm_order",           // node_id of the destination node
      "trigger": "direct"              // "direct" or "event"
    }
  ],
  "start_node": "draft_order",        // must reference a node_id in "nodes"
  "terminal_nodes": ["confirm_order"],// must reference node_ids in "nodes"
  "tags": ["orders", "example"],
  "governing_spec": "007-workflow-registry-traversal"  // must be this exact string
}
```

---

## Direct vs Event Edge

### Direct edge

The runtime advances immediately when the source node completes successfully.

```json
{
  "edge_id": "draft_to_confirm",
  "from": "draft_order",
  "to": "confirm_order",
  "trigger": "direct"
}
```

Use `direct` for purely sequential flows where the downstream node does not depend on an event being emitted.

### Event edge

The runtime waits for the source node to emit a specific governed event before advancing to the destination node.

```json
{
  "edge_id": "draft_to_confirm",
  "from": "draft_order",
  "to": "confirm_order",
  "trigger": "event",
  "event": {
    "event_id": "acme.orders.order-drafted",
    "version": "1.0.0"
  }
}
```

For an event edge to pass validation:

1. The event contract `acme.orders.order-drafted@1.0.0` must be registered in the registry.
2. The source capability (`acme.orders.draft-order`) must declare the event in its `emits` array.

Event-driven edge semantics are governed by spec `018-event-driven-composition`.

---

## Data Flow Between Nodes

Values move between nodes through **workflow state**:

1. The source node's `output.to_workflow_state` names the fields it writes.
2. The destination node's `input.from_workflow_input` names the fields it reads.

The runtime merges fields into a single flat state map as each node completes. A node reads from that map — it does not matter whether the value came from the original workflow input or from an upstream node.

If a required field is missing from state when a node starts, the runtime returns a structured failure rather than proceeding with incomplete input.

---

## Start and Terminal Nodes

`start_node` is the single entry point. The runtime always begins at this node.

`terminal_nodes` is the list of exit points. When any terminal node completes successfully, the workflow is considered done and the runtime returns its output.

Rules:

- `start_node` must reference a `node_id` in `nodes`.
- Every entry in `terminal_nodes` must reference a `node_id` in `nodes`.
- In v0.1, cycles are not permitted. Every node may appear only in one path from `start_node` to a terminal node.

---

## Validate and Register

**Inspect the workflow** (validates structure without modifying state):

```bash
cargo run -p traverse-cli-rs -- workflow inspect \
  workflows/path/to/your-workflow/workflow.json
```

**Register through a bundle** (required before the workflow can be executed):

Create a registry bundle manifest that includes the workflow and all capability contracts it references. Then:

```bash
cargo run -p traverse-cli-rs -- bundle inspect \
  examples/your-bundle/registry-bundle/manifest.json

cargo run -p traverse-cli-rs -- bundle register \
  examples/your-bundle/registry-bundle/manifest.json
```

See [`docs/workflow-composition-guide.md`](workflow-composition-guide.md) for the full bundle manifest shape.

**Run the spec-alignment gate** before opening a PR:

```bash
bash scripts/ci/spec_alignment_check.sh
bash scripts/ci/repository_checks.sh
```

---

## Common Mistakes

- **`governing_spec` mismatch** — the field must equal exactly `007-workflow-registry-traversal`. Any other value causes registration failure with `InvalidLiteral`.
- **Capability not registered** — the workflow references a `capability_id` that does not exist in the registry at the declared version. Register the bundle (which includes the capabilities) before inspecting the workflow.
- **`from_workflow_input` references an undeclared field** — if a node expects a field that was never written to workflow state by an upstream node, runtime execution fails with a missing-input error. Verify the output `to_workflow_state` of the upstream node produces the field the downstream node expects.
- **Event edge without matching `emits`** — an event edge requires the source capability to declare the event in its `emits` array. The registration validator checks this.
- **Cycles in the graph** — v0.1 does not allow cycles. Use predicates on event edges for conditional branching rather than loops.
- **`ImmutableVersionConflict` on re-registration** — once a `(id, version)` triple is registered with a content digest, it cannot be updated. Bump the `version` field to a new semver value when you need to change the workflow.

---

## Related Documents

- [`docs/workflow-composition-guide.md`](workflow-composition-guide.md) — full two-capability worked example
- [`docs/capability-contract-authoring-guide.md`](capability-contract-authoring-guide.md) — how to write the capabilities referenced in nodes
- [`docs/event-contract-authoring-guide.md`](event-contract-authoring-guide.md) — how to write event contracts for event edges
- [`specs/007-workflow-registry-traversal/spec.md`](../specs/007-workflow-registry-traversal/spec.md) — governing spec
- [`specs/018-event-driven-composition/spec.md`](../specs/018-event-driven-composition/spec.md) — event-driven edge semantics

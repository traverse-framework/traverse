# Traverse Getting Started

This guide is the first developer path through Traverse.

Use it when you want to understand one governed capability end to end:

- where the contract lives
- how Traverse packages and registers it
- how the runtime invokes the approved flow
- what output and trace evidence to expect

This is intentionally narrower than [quickstart.md](../quickstart.md). `quickstart.md` starts from the first app-consumable browser flow. This guide starts one layer earlier, at the capability and registry level.

## Prerequisites

- Rust 1.94 or later
- the repository checked out locally
- a shell that can run the checked-in validation scripts

From the repository root, confirm the workspace builds:

```bash
bash scripts/validate-setup.sh
cargo build
```

## The First Capability We Will Follow

Traverse already includes one governed example domain: expedition planning.

For the first-capability path, focus on these files:

- capability contract:
  - `contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json`
- emitted event contract:
  - `contracts/examples/expedition/events/expedition-objective-captured/contract.json`
- registry bundle manifest:
  - `examples/expedition/registry-bundle/manifest.json`
- workflow that composes the expedition flow:
  - `workflows/examples/expedition/plan-expedition/workflow.json`
- canonical runtime request:
  - `examples/expedition/runtime-requests/plan-expedition.json`

This capability is a good starting point because it is simple, governed, and already wired into the approved expedition workflow.

## Step 1: Read The Contract

Open the contract file first:

```bash
sed -n '1,220p' contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json
```

The key fields to notice are:

- `id`, `namespace`, and `version`
- `summary` and `description`
- `inputs.schema`
- `outputs.schema`
- `preconditions` and `postconditions`
- `emits`
- `execution`
- `provenance`

Those fields are the governed source of truth. The runtime and registries conform to them; they do not redefine them.

The current supported CLI surfaces do not expose a standalone `capability inspect` command yet. For this first-capability path, read the contract file directly and treat it as the source of truth.

What you should see in the file:

- `id: expedition.planning.capture-expedition-objective`
- `version: 1.0.0`
- `binary_format: wasm`
- the emitted event reference for `expedition.planning.expedition-objective-captured`

## Step 2: Understand The Implementation Boundary

The expedition example is governed through contracts, workflow artifacts, and a registry bundle. For authoring a new executable capability package, Traverse keeps a minimal template here:

- `examples/templates/executable-capability-package/manifest.template.json`
- `examples/templates/executable-capability-package/src/implementation.rs`
- `examples/templates/executable-capability-package/build-fixture.sh`

Open the template manifest:

```bash
sed -n '1,220p' examples/templates/executable-capability-package/manifest.template.json
```

Open the minimal implementation:

```bash
sed -n '1,120p' examples/templates/executable-capability-package/src/implementation.rs
```

That template shows the execution-side fields a real packaged capability must make explicit:

- `capability_ref`
- `workflow_refs`
- `source`
- `binary`
- `constraints`
- `model_dependencies`

For more authoring depth after this guide, continue with:

- [docs/wasm-agent-authoring-guide.md](./wasm-agent-authoring-guide.md)
- [docs/wasm-microservice-authoring-guide.md](./wasm-microservice-authoring-guide.md)

## Step 3: Inspect The Governed Bundle

Traverse registers the expedition example through one approved bundle manifest:

```bash
cargo run -p traverse-cli-rs -- bundle inspect \
  examples/expedition/registry-bundle/manifest.json
```

This shows how the capability contract participates in a larger governed set.

What you should see in the output:

- `expedition.planning.capture-expedition-objective`
- `expedition.planning.plan-expedition`
- the expedition event contracts
- the expedition workflow entry

## Step 4: Register The Capability Set

Register the approved expedition bundle:

```bash
cargo run -p traverse-cli-rs -- bundle register \
  examples/expedition/registry-bundle/manifest.json
```

What good output looks like:

- `registered_capabilities: 6`
- `registered_events: 5`
- `registered_workflows: 1`
- `expedition.planning.plan-expedition@1.0.0 (workflow)`

This is the step that makes the capability discoverable through Traverse instead of leaving it as a file on disk only.

## Step 5: Inspect The Workflow That Uses It

The first capability is not most useful in isolation. It becomes meaningful as part of the approved expedition workflow:

```bash
cargo run -p traverse-cli-rs -- workflow inspect \
  workflows/examples/expedition/plan-expedition/workflow.json
```

What you should see:

- `id: expedition.planning.plan-expedition`
- `start_node: capture_objective`
- the ordered node capability list

That `start_node` is the first capability you inspected earlier.

## Step 6: Invoke The Canonical Runtime Request

Run the approved expedition request:

```bash
cargo run -p traverse-cli-rs -- expedition execute \
  examples/expedition/runtime-requests/plan-expedition.json
```

This is the first full capability execution path a new Traverse developer should be able to reproduce.

What good output looks like:

- `capability_id: expedition.planning.plan-expedition`
- `status: completed`
- `recommended_route_style: conservative-alpine-push`
- `trace_ref: trace_exec_expedition-plan-request-001`

Even though the request invokes the workflow-backed `plan-expedition` capability, the earlier `capture-expedition-objective` contract is part of the governed execution path you just ran.

## Step 7: Persist And Inspect Trace Evidence

Traverse is built to keep runtime decisions explainable. Persist the trace:

```bash
tmpdir="$(mktemp -d)"
cargo run -p traverse-cli-rs -- expedition execute \
  examples/expedition/runtime-requests/plan-expedition.json \
  --trace-out "$tmpdir/plan-expedition-trace.json"
```

Then inspect it:

```bash
cargo run -p traverse-cli-rs -- trace inspect \
  "$tmpdir/plan-expedition-trace.json"
```

What you should see:

- `trace_id: trace_exec_expedition-plan-request-001`
- `result_status: completed`
- `selected_capability_id: expedition.planning.plan-expedition`

## Validation

Run the canonical expedition example smoke and repository checks:

```bash
bash scripts/validate-setup.sh
bash scripts/ci/expedition_golden_path.sh
bash scripts/ci/repository_checks.sh
```

## What You Learned

After this guide, you should be able to answer:

- where a capability contract lives
- what fields define its governed behavior
- how Traverse groups contracts into a registry bundle
- how the runtime discovers and invokes the approved path
- where trace evidence comes from after execution

## Next Steps

Use these in order:

1. [examples/hello-world/README.md](../examples/hello-world/README.md) for the smallest runnable Traverse example
2. [quickstart.md](../quickstart.md) for the first browser-consumable flow
3. [docs/expedition-example-authoring.md](./expedition-example-authoring.md) for the full governed expedition artifact set
4. [docs/wasm-agent-authoring-guide.md](./wasm-agent-authoring-guide.md) for packaged WASM agent authoring
5. [docs/wasm-microservice-authoring-guide.md](./wasm-microservice-authoring-guide.md) for packaged WASM microservice authoring

If a local command or CI check fails while you work through those paths, use [docs/troubleshooting.md](./troubleshooting.md).

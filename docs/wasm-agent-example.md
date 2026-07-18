# WASM AI Agent Example

Traverse's first governed WASM AI agent example packages the AI-assisted expedition capability:

- package: `examples/agents/expedition-intent-agent/manifest.json`
- approved capability: `expedition.planning.interpret-expedition-intent`
- approved workflow interaction: `expedition.planning.plan-expedition`

Build the deterministic local WASM fixture:

```bash
bash examples/agents/expedition-intent-agent/build-fixture.sh
```

Inspect the governed package:

```bash
cargo run -p traverse-cli-rs -- agent inspect examples/agents/expedition-intent-agent/manifest.json
```

Execute the agent through the Traverse runtime model:

```bash
cargo run -p traverse-cli-rs -- agent execute \
  examples/agents/expedition-intent-agent/manifest.json \
  examples/agents/runtime-requests/interpret-expedition-intent.json
```

Run the deterministic smoke path:

```bash
bash scripts/ci/wasm_agent_example_smoke.sh
```

What it validates:

- the agent package is governed by an approved capability contract with `binary_format: wasm`
- the package declares no host API, network, or filesystem exceptions
- the built WASM artifact matches the declared digest
- the packaged agent executes through the approved Traverse runtime request path
- the agent stays attached to approved expedition capability and workflow surfaces

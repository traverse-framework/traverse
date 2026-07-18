# Second WASM AI Agent Example

Traverse's second governed WASM AI agent example packages the expedition readiness validation capability:

- package: `examples/agents/team-readiness-agent/manifest.json`
- approved capability: `expedition.planning.validate-team-readiness`
- approved workflow interaction: `expedition.planning.plan-expedition`

Build the deterministic local WASM fixture:

```bash
bash examples/agents/team-readiness-agent/build-fixture.sh
```

Inspect the governed package:

```bash
cargo run -p traverse-cli-rs -- agent inspect examples/agents/team-readiness-agent/manifest.json
```

Execute the agent through the Traverse runtime model:

```bash
cargo run -p traverse-cli-rs -- agent execute \
  examples/agents/team-readiness-agent/manifest.json \
  examples/agents/runtime-requests/validate-team-readiness.json
```

Run the deterministic smoke path:

```bash
bash scripts/ci/wasm_agent_team_readiness_smoke.sh
```

What it validates:

- the agent package is governed by an approved capability contract with `binary_format: wasm`
- the package declares no host API, network, or filesystem exceptions
- the built WASM artifact matches the declared digest
- the packaged agent executes through the approved Traverse runtime request path
- the second agent exercises a distinct expedition capability path from the first governed WASM AI agent example

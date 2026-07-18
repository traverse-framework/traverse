# Expedition Example Registry Bundle

The expedition example registry bundle lives at:

```text
examples/expedition/registry-bundle/manifest.json
```

Use the current repo CLI to inspect it:

```bash
cargo run -p traverse-cli-rs -- bundle inspect examples/expedition/registry-bundle/manifest.json
```

What this walkthrough proves:

- the bundle points at the canonical expedition capability contracts
- the bundle points at the canonical expedition event contracts
- the bundle points at the canonical `plan-expedition` workflow artifact
- the governed capability ids and workflow id match the approved expedition specs

Expected ids in the output include:

- `expedition.planning.capture-expedition-objective`
- `expedition.planning.interpret-expedition-intent`
- `expedition.planning.assess-conditions-summary`
- `expedition.planning.validate-team-readiness`
- `expedition.planning.assemble-expedition-plan`
- `expedition.planning.plan-expedition`

Validation commands:

```bash
cargo run -p traverse-cli-rs -- bundle inspect examples/expedition/registry-bundle/manifest.json
bash scripts/ci/expedition_artifact_smoke.sh
bash scripts/ci/repository_checks.sh
```

The walkthrough intentionally uses the canonical artifact ids and paths already governed by the expedition specs.

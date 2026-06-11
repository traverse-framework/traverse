# youaskm3 Integration Validation

This document defines the first real downstream `youaskm3` integration path against Traverse.

It stays on governed public Traverse surfaces only:

- the live local browser adapter
- the checked-in React browser demo
- the downstream MCP consumption validation path
- the first app-consumable quickstart
- the versioned Traverse consumer bundle
- the real-agent MCP exercise guide
- the published-artifact validation path for the released Traverse runtime and MCP artifacts
- the canonical HTTP/JSON app path at [docs/youaskm3-canonical-app-http-path.md](youaskm3-canonical-app-http-path.md)
- [docs/youaskm3-published-artifact-validation.md](youaskm3-published-artifact-validation.md)
- [docs/youaskm3-compatibility-conformance-suite.md](youaskm3-compatibility-conformance-suite.md)
- the `youaskm3` compatibility conformance suite
- [docs/youaskm3-real-shell-validation.md](youaskm3-real-shell-validation.md)
- the `youaskm3` real shell validation

For the shortest Traverse-side start path, begin with [quickstart.md](../quickstart.md).

For the first release-facing HTTP/JSON app path, use [docs/youaskm3-canonical-app-http-path.md](youaskm3-canonical-app-http-path.md).

For the single Traverse `v0.3.0` downstream validation path that `youaskm3` can cite for release evidence, use [docs/v0.3.0-downstream-validation-path.md](v0.3.0-downstream-validation-path.md).

For the first-release readiness index that ties the canonical paths, compatibility statement, packaging expectations, and validation evidence together, use [docs/youaskm3-v0.3.0-integration-readiness.md](youaskm3-v0.3.0-integration-readiness.md).

## Governing Spec

- `specs/019-downstream-consumer-contract/spec.md`
- `specs/020-downstream-integration-validation/spec.md`
- `specs/021-app-facing-operational-constraints/spec.md`
- `specs/023-downstream-publication-strategy/spec.md`

## Purpose

Use one deterministic repo-local validation flow to prove that `youaskm3` can consume Traverse through the documented public surfaces without private repo knowledge or undocumented setup.

## Prerequisites

- A local Traverse checkout with the approved browser adapter, quickstart, and MCP validation docs available.
- Rust and Node.js installed locally.
- Optional: a sibling `youaskm3` checkout if you want to run the downstream app alongside Traverse while following the documented flow.

## Traverse Validation Path

Run the browser and MCP validation smoke checks in order:

```bash
bash scripts/ci/react_demo_live_adapter_smoke.sh
bash scripts/ci/mcp_consumption_validation.sh
```

Then run the integration validation wrapper:

```bash
bash scripts/ci/youaskm3_integration_validation.sh
```

For the broader release-aligned compatibility check, also run:

```bash
bash scripts/ci/youaskm3_compatibility_conformance.sh
```

For the pinned Traverse `v0.3.0` release evidence sequence, follow [docs/v0.3.0-downstream-validation-path.md](v0.3.0-downstream-validation-path.md).

For the published-artifact validation against the released Traverse runtime and MCP artifacts, also run:

```bash
bash scripts/ci/youaskm3_published_artifact_validation.sh
```

For the real browser-hosted shell validation against released Traverse consumer artifacts, also run:

```bash
bash scripts/ci/youaskm3_real_shell_validation.sh
```

For the real agent exercise against the documented MCP substrate, also run:

```bash
bash scripts/ci/mcp_real_agent_exercise_smoke.sh
```

## Expected Evidence

The validation path should prove:

- request submission from the approved public consumer path
- ordered runtime updates and terminal outcome consumption
- trace visibility through the public Traverse surfaces
- `consumer_name: youaskm3`
- `validated_flow_id: youaskm3_mcp_validation`
- no dependency on private Traverse internals or undocumented setup
- the same Traverse v0.1 release pairing used by the compatibility conformance suite
- the browser-hosted shell spec at `openspec/specs/pwa-shell/spec.md`
- the downstream repo-local smoke validation at `scripts/smoke.sh`
- the real-agent MCP exercise at `docs/mcp-real-agent-exercise.md`

## Known Failure Modes

The path is expected to fail deterministically when:

- the live browser adapter is unavailable
- the MCP consumption surface is unavailable
- the documented quickstart or validation docs are missing
- the downstream consumer path cannot be followed through the public surfaces

## Validation

- `bash scripts/ci/react_demo_live_adapter_smoke.sh`
- `bash scripts/ci/mcp_consumption_validation.sh`
- `bash scripts/ci/mcp_real_agent_exercise_smoke.sh`
- `bash scripts/ci/youaskm3_integration_validation.sh`
- `bash scripts/ci/repository_checks.sh`

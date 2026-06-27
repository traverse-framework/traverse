# Traverse Spec Routing Guide

Traverse uses split, focused governing specs instead of one giant spec file. This keeps the context load small for agents and reviewers: start here, open only the owning spec, then follow the listed dependencies when the change crosses a boundary.

## Routing Rules

- Start with the row that owns the behavior you are changing.
- Read the owning `spec.md` before implementation.
- Read `context.md` when it exists; it summarizes invariants, non-ownership, and issue mappings for low-context agent work.
- Do not duplicate rules across specs. Cross-reference the owning spec instead.
- If two specs appear to own the same rule, update this guide or create a clarification ticket before implementation.

## App-Consumable Runtime Cluster

| Question | Owning spec | Context file |
| --- | --- | --- |
| HTTP endpoints, response envelopes, errors, CORS, OpenAPI, local server discovery | `033-http-json-api` | `specs/033-http-json-api/context.md` |
| Capability, event contract, workflow, and bundle registration semantics | `034-programmatic-registration` | `specs/034-programmatic-registration/context.md` |
| Workspace identity, bearer auth, scopes, runtime grants, and audit log rules | `035-multi-agent-isolation` | `specs/035-multi-agent-isolation/context.md` |

## Strategic Spec Ownership

| Question | Owning spec |
| --- | --- |
| Integrated telemetry, traces, metrics, log semantics | `029-integrated-observability` |
| Security model, trust boundaries, identity direction | `030-security-identity-model` |
| Supply chain hardening, provenance, artifact trust | `031-supply-chain-hardening` |
| Portable state and data access constraints | `032-universal-data-access` |
| Event delivery, replay, and missed-event recovery | `036-event-subscription-replay` |
| Semver range matching and compatible version selection | `037-semver-range-resolution` |
| WASI and host ABI insulation from standards churn | `038-wasi-host-insulation` |
| Connector/plugin model and third-party integrations | `039-connector-plugin-architecture` |
| Contractual enforcement gates and fail-fast validation | `040-contractual-enforcement-gate` |
| Programmatic workflow building and composition | `041-workflow-composition-api` |
| MCP library surface and embeddable agent APIs | `042-mcp-library-surface` |
| Module dependency registry and resolution strategy | `043-module-dependency-management` |
| Downstream app manifests, component manifests, app config, and app/component CLI scaffolding | `044-application-bundle-manifest` |
| Real model dependency candidates, local inference readiness, selection heuristics, and model selection trace evidence | `045-governed-model-dependency-resolution` |
| Public CLI app validation/registration, durable local workspace state, and runtime loading of registered app bundles | `046-public-cli-app-registration` |

## Context Hygiene

Implementation agents should prefer this read order:

1. `specs/README.md`
2. owning `context.md`, when present
3. owning `spec.md`
4. directly referenced specs only
5. related implementation ticket and PR

This pattern is intentionally boring. Boring here means fewer accidental cross-spec rewrites, smaller LLM prompts, and less archaeology before code can move.

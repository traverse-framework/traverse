[![Traverse](https://github.com/user-attachments/assets/479aa1e5-5799-4c7d-bb8a-4b30d711c7db)](https://traverse-framework.com/)


# Traverse

[![CI](https://github.com/enricopiovesan/Traverse/actions/workflows/ci.yml/badge.svg)](https://github.com/enricopiovesan/Traverse/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-100%25-brightgreen)](https://github.com/enricopiovesan/Traverse/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange)](https://www.rust-lang.org/)
[![Version](https://img.shields.io/badge/version-v0.5.0-blue)](https://github.com/enricopiovesan/Traverse/releases)

Your business logic runs in the browser, on your server, and in a cloud function.
They drift. You maintain three versions of the same behavior.
Traverse keeps it in one contract and runs it anywhere — with a full execution trace every time.

Traverse is the working implementation of [Universal Microservices Architecture](https://www.universalmicroservices.com/).

---

## Quick Start

**Requirements**: Rust 1.94+

```bash
git clone https://github.com/enricopiovesan/Traverse.git
cd Traverse
cargo build
cargo run -p traverse-cli -- bundle inspect examples/expedition/registry-bundle/manifest.json
```

Expected output:

```
bundle_id: expedition.planning.seed-bundle
version: 1.0.0
capabilities: 6
events: 5
workflows: 1
```

You just inspected a live capability bundle — 6 capabilities, 5 events, 1 workflow, all defined in contracts that the runtime validates and executes.

Ready to run the full browser demo? → [quickstart.md](quickstart.md)

---

## What Can I Build?

### A browser app with governed runtime behavior

Build the UI. Traverse owns execution, workflow state, and trace output.
The same capability contract runs locally in development and on the edge in production — no rewrite.

→ [quickstart.md](quickstart.md) · [docs/app-consumable-entry-path.md](docs/app-consumable-entry-path.md)

### A governed MCP server

Expose capability discovery and execution over stdio. Downstream AI clients and tools
discover and call governed capabilities without touching your internals.

→ [docs/mcp-stdio-server.md](docs/mcp-stdio-server.md)

### Portable WASM capabilities

Package executable behavior as WASM. Traverse validates, places, and runs it —
browser, edge, or cloud — under the same contract.

→ [docs/wasm-agent-authoring-guide.md](docs/wasm-agent-authoring-guide.md)

### Workflow-backed business logic

Model multi-step business behavior as a workflow. The runtime traverses it
deterministically and produces a structured trace you can inspect and audit.

→ [docs/workflow-composition-guide.md](docs/workflow-composition-guide.md) · [docs/getting-started.md](docs/getting-started.md)

### Your own app bundle

```bash
cargo run -p traverse-cli -- app new my-app
```

Scaffolds a governed app bundle. Add your capability contracts, workflows, and WASM components.

→ [docs/expedition-example-authoring.md](docs/expedition-example-authoring.md)

---

## Documentation

### Onboarding and tutorial path

| Goal | Start here | Continue with |
|---|---|---|
| First runnable flow | [quickstart.md](quickstart.md) | [docs/app-consumable-entry-path.md](docs/app-consumable-entry-path.md) |
| Learn the core path | [docs/getting-started.md](docs/getting-started.md) | [docs/expedition-example-authoring.md](docs/expedition-example-authoring.md) |
| Author a capability contract | [docs/capability-contract-authoring-guide.md](docs/capability-contract-authoring-guide.md) | [docs/getting-started.md](docs/getting-started.md) |
| Author an event contract | [docs/event-contract-authoring-guide.md](docs/event-contract-authoring-guide.md) | [docs/event-publishing-tutorial.md](docs/event-publishing-tutorial.md) |
| Build WASM capabilities | [docs/wasm-agent-authoring-guide.md](docs/wasm-agent-authoring-guide.md) | [docs/wasm-microservice-authoring-guide.md](docs/wasm-microservice-authoring-guide.md) |
| Integrate a downstream app | [docs/app-consumable-consumer-bundle.md](docs/app-consumable-consumer-bundle.md) | [docs/youaskm3-integration-validation.md](docs/youaskm3-integration-validation.md) |
| Troubleshoot a failure | [docs/troubleshooting.md](docs/troubleshooting.md) | [docs/quality-standards.md](docs/quality-standards.md) |

### Consumer and release surfaces

- [docs/releases/v0.5.0.md](docs/releases/v0.5.0.md) — current release notes
- [docs/app-consumable-consumer-bundle.md](docs/app-consumable-consumer-bundle.md) — versioned consumer bundle
- [docs/app-consumable-package-release-pointer.md](docs/app-consumable-package-release-pointer.md) — package release pointer
- [docs/packaged-traverse-runtime-artifact.md](docs/packaged-traverse-runtime-artifact.md) — packaged runtime artifact
- [docs/packaged-traverse-mcp-server-artifact.md](docs/packaged-traverse-mcp-server-artifact.md) — packaged MCP server artifact
- [docs/youaskm3-canonical-app-http-path.md](docs/youaskm3-canonical-app-http-path.md) — canonical HTTP app path
- [docs/youaskm3-canonical-mcp-client-path.md](docs/youaskm3-canonical-mcp-client-path.md) — canonical MCP client path
- [docs/youaskm3-integration-validation.md](docs/youaskm3-integration-validation.md) — youaskm3 integration validation
- [docs/youaskm3-published-artifact-validation.md](docs/youaskm3-published-artifact-validation.md) — published-artifact validation
- [docs/youaskm3-compatibility-conformance-suite.md](docs/youaskm3-compatibility-conformance-suite.md) — compatibility conformance suite
- [docs/youaskm3-real-shell-validation.md](docs/youaskm3-real-shell-validation.md) — real shell validation
- [docs/mcp-real-agent-exercise.md](docs/mcp-real-agent-exercise.md) — real AI agent exercise for the MCP surface

### v0.3.0 consumer paths

- [docs/v0.3.0-public-surface-compatibility.md](docs/v0.3.0-public-surface-compatibility.md) — v0.3.0 public surface compatibility
- [docs/v0.3.0-source-build-consumer-packaging.md](docs/v0.3.0-source-build-consumer-packaging.md) — source-build packaging for v0.3.0 consumers
- [docs/v0.3.0-downstream-validation-path.md](docs/v0.3.0-downstream-validation-path.md) — downstream validation path for v0.3.0
- [docs/youaskm3-v0.3.0-integration-readiness.md](docs/youaskm3-v0.3.0-integration-readiness.md) — v0.3.0 integration readiness index

### Reference

- [docs/adapter-boundaries.md](docs/adapter-boundaries.md) — adapter and portability boundaries
- [docs/compatibility-policy.md](docs/compatibility-policy.md) — versioning and compatibility
- [docs/troubleshooting.md](docs/troubleshooting.md) — shortest path through common failures
- [docs/what-can-i-build.md](docs/what-can-i-build.md) — concrete app and integration patterns
- [docs/benchmarks.md](docs/benchmarks.md) — measured latency numbers
- [docs/decision-log.md](docs/decision-log.md) — consolidated architecture decisions

---

## Architecture

### Crates

| Crate | Role |
|---|---|
| `traverse-runtime` | Core execution engine — validates, places, and executes capabilities |
| `traverse-contracts` | Contract definitions, parsing, and validation |
| `traverse-registry` | Capability and event registries with deterministic traversal |
| `traverse-cli` | Command-line interface: register, list, validate, run |
| `traverse-mcp` | Model Context Protocol stdio server and governed MCP-facing surface |

---

## Contributing

Please read before opening a PR:

- [CONTRIBUTING.md](CONTRIBUTING.md)
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [SECURITY.md](SECURITY.md)
- [docs/quality-standards.md](docs/quality-standards.md)

All work follows the governance workflow below. Every PR must be backed by an approved spec.

[GitHub Project 1](https://github.com/users/enricopiovesan/projects/1/) is the canonical board. All active work has an issue, a project item, and a PR.

---

## Built on UMA

Traverse is the runtime that [Universal Microservices Architecture](https://www.universalmicroservices.com/) describes — the answer in working Rust code to the question: *how do you keep one business behavior portable and governed as execution moves across browser, edge, cloud, workflows, and AI?*

| | UMA | Traverse |
|---|---|---|
| What it is | Architecture model + book | Working runtime implementation |
| Business capabilities | Defines the concept | Executes them with contracts and specs |
| Portability | Describes the pattern | Enforces it through WASM and adapters |
| Governance | Specifies the rules | Implements them as immutable specs and CI gates |
| AI safety | Describes requirements | Delivers through explainable runtime traces |

**If you want to understand the ideas:** [read the UMA book](https://www.universalmicroservices.com/) and explore the [UMA code examples](https://github.com/enricopiovesan/UMA-code-examples).

---

## Governance

Traverse is spec-driven. Code must align with an approved, immutable spec or it does not merge.

| Artifact | Location | Role |
|---|---|---|
| Specs | [`specs/`](specs/) | Versioned, immutable, merge-gating |
| Contracts | [`contracts/`](contracts/) | Source of truth for runtime behavior |
| Constitution | [`.specify/memory/constitution.md`](.specify/memory/constitution.md) | Overrides all convenience decisions |
| CI gate | [`scripts/ci/spec_alignment_check.sh`](scripts/ci/spec_alignment_check.sh) | Deterministic, AI-agnostic |

### Approved Specs

| ID | Spec | Governs |
|---|---|---|
| 001 | [foundation-v0-1](specs/001-foundation-v0-1/spec.md) | Core runtime, CLI, MCP surface |
| 002 | [capability-contracts](specs/002-capability-contracts/spec.md) | Contract definitions and validation |
| 003 | [event-contracts](specs/003-event-contracts/spec.md) | Event contract definitions |
| 004 | [spec-alignment-gate](specs/004-spec-alignment-gate/spec.md) | CI merge gate |
| 005 | [capability-registry](specs/005-capability-registry/spec.md) | Registry behavior |
| 006 | [runtime-request-execution](specs/006-runtime-request-execution/spec.md) | Execution model |
| 007 | [workflow-registry-traversal](specs/007-workflow-registry-traversal/spec.md) | Workflow composition |
| 008 | [expedition-example-domain](specs/008-expedition-example-domain/spec.md) | Example domain |
| 009 | [expedition-example-artifacts](specs/009-expedition-example-artifacts/spec.md) | Example artifacts |

---

## For Agents

This project supports AI-assisted development with Codex and Claude Code running in parallel.

### Entry points

| Agent | File | Purpose |
|---|---|---|
| Claude Code | [`CLAUDE.md`](CLAUDE.md) | Project context, governance rules, speckit workflow |
| Codex | [`AGENTS.md`](AGENTS.md) | Project context, coordination rules, speckit workflow |
| All agents | [`.specify/memory/constitution.md`](.specify/memory/constitution.md) | Governing constitution v1.2.0 |

### Agent workflow

1. **Read** your entry point file (`CLAUDE.md` or `AGENTS.md`)
2. **Claim** the ticket — check for `agent:claude` / `agent:codex` labels and existing branches
3. **Create** a feature branch: `NNN-feature-name`
4. **Run** `.specify/scripts/bash/setup-plan.sh --json` to initialize the spec directory
5. **Write** `specs/<branch>/spec.md` and `plan.md` before any code
6. **Implement** the smallest change that satisfies the spec and contracts
7. **Open** a PR with `## Governing Spec`, `## Project Item`, and `## Validation` sections

### Agent coordination

- `agent:claude` label = claimed by Claude Code — Codex must skip
- `agent:codex` label = claimed by Codex — Claude Code must skip
- Full coordination rules: [`docs/multi-thread-workflow.md`](docs/multi-thread-workflow.md)

---

## License

Apache-2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

## Related Work

- [UMA-code-examples](https://github.com/enricopiovesan/UMA-code-examples)
- [Universal Microservices Architecture — Book](https://www.amazon.com/dp/B0GTTTTQH4)
- [Contract-Driven AI Development (C-DAD) — White Paper](https://drive.google.com/file/d/1HC_ZWJl9aYaMeN78qiL3ZYBVY7mAGl3f/view)
- [Speaking](https://enricopiovesan.github.io/enricopiovesan/)
- [github.com/enricopiovesan](https://github.com/enricopiovesan)

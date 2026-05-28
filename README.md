[![Traverse](https://github.com/user-attachments/assets/aeafaaf8-650d-4489-bf5e-bd386f0bcaf0)](https://enricopiovesan.com/)


# Traverse

[![CI](https://github.com/enricopiovesan/Traverse/actions/workflows/ci.yml/badge.svg)](https://github.com/enricopiovesan/Traverse/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-100%25-brightgreen)](https://github.com/enricopiovesan/Traverse/actions/workflows/ci.yml)
[![Spec Governed](https://img.shields.io/badge/spec-governed-blueviolet)](specs/governance/approved-specs.json)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange)](https://www.rust-lang.org/)
[![Version](https://img.shields.io/badge/version-v0.1--rc-lightgrey)](https://github.com/enricopiovesan/Traverse/releases)



**Run one governed business capability across browser, edge, and cloud without rewriting it.**

Traverse is a contract-driven runtime for discovering, validating, and composing portable business capabilities through events, policies, constraints, and graph-based workflows.

## Why Should I Care?

Most systems are portable at the container boundary, but not at the business behavior boundary. The moment you need the same governed behavior to run in multiple hosts (browser, edge, cloud, device), you typically end up with duplicated implementations and behavior drift.

Traverse is built to make the capability contract the source of truth, and make execution traceable and governed regardless of where it runs.

**Killer use case (first target):** a portable knowledge workflow that runs offline in the browser and can also run in cloud/edge contexts, without splitting the business behavior into separate implementations. See: [docs/killer-use-case-portable-knowledge-app.md](docs/killer-use-case-portable-knowledge-app.md).

### What’s Proven Today

- A spec-governed runtime with deterministic CI gates and versioned governing specs
- “App-consumable” release surfaces with downstream validation (see: [docs/app-consumable-entry-path.md](docs/app-consumable-entry-path.md) and [docs/youaskm3-integration-validation.md](docs/youaskm3-integration-validation.md))

### Who It’s For (and Not For)

Traverse is for teams that must run the same governed behavior across multiple host environments and want contract-first portability, traceability, and governance.

Traverse is not a replacement for Docker-orchestrated services when “portable enough” means “runs in a container in one environment”, and you do not need host-level portability or capability-level governance.

This is personal research and development by [Enrico Piovesan](https://enricopiovesan.com), built to prove in code the ideas behind [Universal Microservices Architecture (UMA)](https://github.com/enricopiovesan/UMA-code-examples).

---

## Built on UMA

Traverse is the runtime that [Universal Microservices Architecture](https://www.universalmicroservices.com/) describes.

UMA answers the question: *how do you keep one business behavior portable and governed as execution moves across browser, edge, cloud, workflows, and AI?* Traverse is the answer in working Rust code — contracts, registries, a governed runtime, and structured traces.

| | UMA | Traverse |
|---|---|---|
| What it is | Architecture model + book | Working runtime implementation |
| Business capabilities | Defines the concept | Executes them with contracts and specs |
| Portability | Describes the pattern | Enforces it through WASM and adapters |
| Governance | Specifies the rules | Implements them as immutable specs and CI gates |
| AI safety | Describes requirements | Delivers through explainable runtime traces |

**If you want to understand the ideas:** [read the UMA book](https://www.universalmicroservices.com/) and explore the [UMA code examples](https://github.com/enricopiovesan/UMA-code-examples).

**If you want to run them:** you're in the right place.

---

## For Humans

### Quick Start

```bash
git clone https://github.com/enricopiovesan/Traverse.git
cd Traverse

cargo build                   # build all crates
cargo test                    # run the full test suite
cargo run -p traverse-cli     # run the CLI
```

**Requirements**: Rust 1.94+

New to Traverse? Start with **[docs/tutorial-index.md](docs/tutorial-index.md)**. That page describes three paths and tells you which one fits your goal.

### Documentation Map

Use this as the human and agent navigation hub for the supported docs:

| Goal | Start Here | Continue With |
|---|---|---|
| Learn the core Traverse path | [docs/getting-started.md](docs/getting-started.md) | [docs/expedition-example-authoring.md](docs/expedition-example-authoring.md) |
| Follow the full onboarding sequence | [docs/tutorial-index.md](docs/tutorial-index.md) | [quickstart.md](quickstart.md) |
| Run the first app-consumable flow | [quickstart.md](quickstart.md) | [docs/app-consumable-entry-path.md](docs/app-consumable-entry-path.md) |
| Author your first capability contract | [docs/capability-contract-authoring-guide.md](docs/capability-contract-authoring-guide.md) | [docs/getting-started.md](docs/getting-started.md) |
| Author your first event contract | [docs/event-contract-authoring-guide.md](docs/event-contract-authoring-guide.md) | [docs/event-publishing-tutorial.md](docs/event-publishing-tutorial.md) |
| Build WASM-hosted capabilities | [docs/wasm-agent-authoring-guide.md](docs/wasm-agent-authoring-guide.md) | [docs/wasm-microservice-authoring-guide.md](docs/wasm-microservice-authoring-guide.md) |
| Integrate a downstream app like `youaskm3` | [docs/app-consumable-consumer-bundle.md](docs/app-consumable-consumer-bundle.md) | [docs/youaskm3-integration-validation.md](docs/youaskm3-integration-validation.md) |
| Review runtime and MCP release surfaces | [docs/packaged-traverse-runtime-artifact.md](docs/packaged-traverse-runtime-artifact.md) | [docs/packaged-traverse-mcp-server-artifact.md](docs/packaged-traverse-mcp-server-artifact.md) |
| Review standards, workflow, and governance | [docs/quality-standards.md](docs/quality-standards.md) | [docs/project-management.md](docs/project-management.md) |

### What it does

- Define a **capability contract** — one meaningful business action with explicit inputs, outputs, and side effects
- **Register** it in the capability registry
- **Validate** it against its governing spec
- **Execute** it locally through the runtime with structured trace output

### Vision

Traverse treats business capabilities as the primary unit of software:

- portable across browser, edge, cloud, and device
- governed through versioned, immutable contracts and specs
- composable through events and graph-based workflows
- explainable through structured runtime traces
- safe for humans, runtimes, and AI systems to consume

### Contributing

Please read before opening a PR:

- [CONTRIBUTING.md](CONTRIBUTING.md)
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [SECURITY.md](SECURITY.md)
- [docs/quality-standards.md](docs/quality-standards.md)

All work follows the [Speckit governance workflow](#governance). Every PR must be backed by an approved spec.

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

### Approved specs

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

## Architecture

### Crates

| Crate | Role |
|---|---|
| `traverse-runtime` | Core execution engine — validates, places, and executes capabilities |
| `traverse-contracts` | Contract definitions, parsing, and validation |
| `traverse-registry` | Capability and event registries with deterministic traversal |
| `traverse-cli` | Command-line interface: register, list, validate, run |
| `traverse-mcp` | Model Context Protocol stdio server and governed MCP-facing surface |

### Governance

Traverse is spec-driven. Code must align with an approved, immutable spec or it does not merge.

| Artifact | Location | Role |
|---|---|---|
| Specs | [`specs/`](specs/) | Versioned, immutable, merge-gating |
| Contracts | [`contracts/`](contracts/) | Source of truth for runtime behavior |
| Constitution | [`.specify/memory/constitution.md`](.specify/memory/constitution.md) | Overrides all convenience decisions |
| CI gate | [`scripts/ci/spec_alignment_check.sh`](scripts/ci/spec_alignment_check.sh) | Deterministic, AI-agnostic |

### Key docs

#### Onboarding and tutorial path

- [docs/tutorial-index.md](docs/tutorial-index.md) — ordered onboarding path for developers and agents
- [docs/getting-started.md](docs/getting-started.md) — first capability path for new developers
- [quickstart.md](quickstart.md) — first runnable app-consumable flow
- [docs/app-consumable-entry-path.md](docs/app-consumable-entry-path.md) — browser-hosted consumer path

#### Build and authoring guides

- [docs/expedition-example-authoring.md](docs/expedition-example-authoring.md) — canonical example authoring flow
- [docs/workflow-composition-guide.md](docs/workflow-composition-guide.md) — beginner guide to chaining two capabilities into a workflow
- [docs/tutorial-index.md](docs/tutorial-index.md) — ordered onboarding path for new developers and agents
- [quickstart.md](quickstart.md) — start here for the first runnable flow
- [docs/app-consumable-entry-path.md](docs/app-consumable-entry-path.md) — first app-consumable flow
- [docs/app-consumable-consumer-bundle.md](docs/app-consumable-consumer-bundle.md) — versioned consumer bundle
- [docs/app-consumable-package-release-pointer.md](docs/app-consumable-package-release-pointer.md) — package release pointer
- [docs/adapter-boundaries.md](docs/adapter-boundaries.md) — adapter and portability boundaries
- [docs/mcp-stdio-server.md](docs/mcp-stdio-server.md) — supported MCP server bootstrap path and command surface
- [docs/wasm-agent-authoring-guide.md](docs/wasm-agent-authoring-guide.md) — how to create new WASM agents
- [docs/wasm-agent-team-readiness-example.md](docs/wasm-agent-team-readiness-example.md) — second governed WASM AI agent example
- [docs/wasm-microservice-authoring-guide.md](docs/wasm-microservice-authoring-guide.md) — how to create new WASM microservices
- [docs/mcp-real-agent-exercise.md](docs/mcp-real-agent-exercise.md) — real AI agent exercise for the Traverse MCP surface

#### Consumer and release surfaces

- [docs/app-consumable-consumer-bundle.md](docs/app-consumable-consumer-bundle.md) — versioned consumer bundle
- [docs/app-consumable-package-release-pointer.md](docs/app-consumable-package-release-pointer.md) — package release pointer
- [docs/event-publishing-tutorial.md](docs/event-publishing-tutorial.md) — how to emit and receive governed events from a capability
- [docs/packaged-traverse-runtime-artifact.md](docs/packaged-traverse-runtime-artifact.md) — packaged runtime artifact
- [docs/packaged-traverse-mcp-server-artifact.md](docs/packaged-traverse-mcp-server-artifact.md) — packaged MCP server artifact
- [docs/youaskm3-integration-validation.md](docs/youaskm3-integration-validation.md) — youaskm3 integration validation
- [docs/youaskm3-published-artifact-validation.md](docs/youaskm3-published-artifact-validation.md) — published-artifact validation for youaskm3
- [docs/youaskm3-compatibility-conformance-suite.md](docs/youaskm3-compatibility-conformance-suite.md) — youaskm3 compatibility conformance suite
- [docs/youaskm3-real-shell-validation.md](docs/youaskm3-real-shell-validation.md) — youaskm3 real shell validation

#### Reference and governance

- [docs/adapter-boundaries.md](docs/adapter-boundaries.md) — adapter and portability boundaries
- [docs/quality-standards.md](docs/quality-standards.md) — non-negotiable quality rules
- [docs/compatibility-policy.md](docs/compatibility-policy.md) — versioning and compatibility
- [docs/troubleshooting.md](docs/troubleshooting.md) — shortest path through common local and CI failures
- [docs/what-can-i-build.md](docs/what-can-i-build.md) — concrete app and integration patterns supported today
- [docs/why-not-docker.md](docs/why-not-docker.md) — when to use Traverse vs Docker (decision matrix)
- [docs/benchmarks.md](docs/benchmarks.md) — measured latency numbers and Docker comparison
- [docs/spec-numbering.md](docs/spec-numbering.md) — how spec ids, paths, and versions relate
- [docs/multi-thread-workflow.md](docs/multi-thread-workflow.md) — parallel agent workflow
- [docs/project-management.md](docs/project-management.md) — issue and board rules
- [docs/spec-reviewer-guide.md](docs/spec-reviewer-guide.md) — reviewer template for governing specs
- [docs/adr/README.md](docs/adr/README.md) — architecture decision records

### Task board

[GitHub Project 1](https://github.com/users/enricopiovesan/projects/1/) is the canonical board. All active work has an issue, a project item, and a PR.

---

## License

Apache-2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

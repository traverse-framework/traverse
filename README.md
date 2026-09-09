[![Traverse](https://github.com/user-attachments/assets/479aa1e5-5799-4c7d-bb8a-4b30d711c7db)](https://traverse-framework.com/)


# Traverse

[![CI](https://github.com/traverse-framework/traverse/actions/workflows/ci.yml/badge.svg)](https://github.com/traverse-framework/traverse/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-core%20100%25-brightgreen)](https://github.com/traverse-framework/traverse/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange)](https://www.rust-lang.org/)
[![Version](https://img.shields.io/badge/version-v0.10.0-blue)](https://github.com/traverse-framework/traverse/releases)
[![Registry](https://img.shields.io/badge/registry-46%20capabilities-6f42c1)](https://registry.traverse-framework.com/)

**Define once. Run anywhere.**

Traverse is a contract-driven WebAssembly runtime for portable business
capabilities. You write a piece of business logic once — as a capability with a
machine-readable contract — and the same signed WASM binary runs on Linux,
macOS, and Windows, on iOS and Android, in the browser, and inside an AI agent
— producing a verifiable execution trace every time.

No reimplementation per environment. No agent free-handing your pricing rules.
One behavior, governed, everywhere it needs to run.

---

## Why this matters

Business logic no longer lives in one place. The same eligibility check,
pricing rule, or approval policy now has to run in a web client, on a server,
at the edge, and — increasingly — inside an AI agent that a user is talking to.
Teams answer that by reimplementing the rule in each stack. The copies drift.
The behavior stops being one thing.

AI coding agents make this sharper, not softer. An agent asked to "add the
discount logic" will re-derive that logic from scratch, in prose, every
session — unversioned, unreviewed, and authoritative only because it ran last.

Traverse takes the other position: **the agent proposes, the runtime decides.**
A capability is a contract (JSON Schema in, JSON Schema out), an immutable
version, a signature, and a WASM artifact. The runtime validates every input
against the contract, isolates execution in a WASM sandbox, enforces policy,
and emits a trace you can audit. An agent's job is to *find and compose* the
right capabilities — not to be the source of truth for what the business does.

This is the working implementation of
[Universal Microservices Architecture](https://www.universalmicroservices.com/):
write once, run where it makes sense, keep the decisions queryable instead of
buried in a framework.

---

## Reuse instead of regenerate

The [public registry](https://registry.traverse-framework.com/) currently holds
**46 capabilities across 24 domains** (117 published versions) — pricing,
authorization, escalation, deadline pressure, completion-quality scoring,
classification, summarization, and more. Every record is contract-defined,
semver'd, signed, and CI-validated before it merges.

Before an agent (or a developer) writes a rule, it can check whether that rule
already exists:

```bash
traverse-cli registry sync   --workspace local-default --json   # pull the index locally
traverse-cli registry search price --workspace local-default --json
traverse-cli registry list   --workspace local-default --json
```

Find `core.calculate-price@1.1.0`, compose it into a workflow, done. What you
*don't* spend tokens or review cycles on:

- re-deriving the input/output contract
- re-implementing and re-testing the logic
- re-reviewing a fresh, unverified copy of a rule the org already agreed on

The same discovery surface is exposed over MCP, so an AI client can list and
call governed capabilities directly. The
[claude-skills](https://github.com/traverse-framework/claude-skills) skill set
makes "check the registry first" the default step, so agents stop duplicating
what's already published.

---

## Quick Start

**Requirements**: Rust 1.94+

```bash
git clone https://github.com/traverse-framework/traverse.git
cd traverse
cargo build
cargo run -p traverse-cli-rs -- bundle inspect examples/expedition/registry-bundle/manifest.json
```

Expected output:

```
bundle_id: expedition.planning.seed-bundle
version: 1.0.0
capabilities: 6
events: 5
workflows: 1
```

You just inspected a live capability bundle — 6 capabilities, 5 events, 1
workflow, all defined in contracts the runtime validates and executes. From
here:

- Run it end to end and see the trace → [docs/getting-started.md](docs/getting-started.md)
- Full browser + HTTP walkthrough → [quickstart.md](quickstart.md)
- Author your own capability → [docs/capability-contract-authoring-guide.md](docs/capability-contract-authoring-guide.md)

---

## Where it runs

One capability contract and one WASM artifact, executed the same way on every
target through a platform embedder that speaks `embedder-api/1.0.0`. Each
embedder digest-verifies the runtime, rejects ambient imports, and enforces the
same Host ABI — the browser is one target among several, not the default.

| Platform | Embedder | WASM host | Status |
|---|---|---|---|
| **Linux / server / CLI** | `traverse-embedder` (Rust) · `traverse-cli` | Wasmtime | Published on crates.io |
| **Browser** | `traverse-embedder-web` (TypeScript) | the browser's own `WebAssembly` | Published (npm) |
| **iOS / macOS** | `packages/swift` — `TraverseEmbedder` Swift Package | WasmKit | In-repo package, CI-conformed; not yet on SwiftPM |
| **Android** | `packages/kotlin` — `TraverseEmbedder` Android library | Chicory | In-repo package, CI-conformed; not yet on Maven |
| **Windows / WinUI** | `packages/dotnet` — `TraverseEmbedder` .NET library | Wasmtime .NET | In-repo package, CI-conformed; not yet on NuGet |
| **AI agent** | `traverse-mcp` stdio server | via the host embedder | Published on crates.io |

All five embedders run against one CI-enforced conformance suite (spec
`068-public-platform-embedder-packages`), so a capability that passes on one
platform behaves the same on the rest. Desktop targets (Linux x86_64/aarch64,
macOS x86_64/arm64, Windows x86_64) are covered by the CI matrix on every PR.

**Cloud and edge** placement targets are specified and on the roadmap, not yet
shipped.

Scaffold your own governed bundle with `traverse-cli app new <id>` —
[docs/expedition-example-authoring.md](docs/expedition-example-authoring.md).
Guides: [docs/wasm-microservice-authoring-guide.md](docs/wasm-microservice-authoring-guide.md) ·
[docs/mcp-stdio-server.md](docs/mcp-stdio-server.md) ·
[docs/mcp-real-agent-exercise.md](docs/mcp-real-agent-exercise.md) ·
[quickstart.md](quickstart.md).

---

## Project state

Traverse is **pre-1.0 (`v0.10.0`)** and spec-driven — every capability below is
real, running, tested code.

| | |
|---|---|
| **Runtime crates** | 8 in this repo; 6 published to [crates.io](https://crates.io/search?q=traverse-) at `0.10.0` (`traverse-contracts`, `traverse-runtime`, `traverse-embedder`, `traverse-mcp`, `traverse-cli-rs`, `traverse-expedition-wasm`). `traverse-native-bridge` and `traverse-swift-host` are newer and not yet published. |
| **Platform SDKs** | 5 embedders on one `embedder-api/1.0.0` contract and one CI conformance suite — Rust (crates.io) and Web/TypeScript (npm) published; Swift/iOS+macOS (WasmKit), Kotlin/Android (Chicory), and .NET/Windows (Wasmtime) in `packages/` with production runtime bridges, not yet on SwiftPM/Maven/NuGet. |
| **Registry** | [`traverse-framework/registry`](https://github.com/traverse-framework/registry) — its own repo (spec 051). `traverse-registry` `0.18.0` on crates.io; **46 capabilities / 117 versions / 24 domains** in the live catalog, all signed and CI-validated. |
| **Governance** | **133 approved, immutable specs** gate the runtime, contracts, registry, MCP surface, WASM execution, native embedding, event delivery, and durable local storage. `jq -r '.specs[].id' specs/governance/approved-specs.json` |
| **Quality bar** | 100% line coverage enforced on the core crates (`traverse-contracts`, `traverse-runtime`, `traverse-embedder`); `traverse-cli-rs` 87%, `traverse-mcp` 98%. Spec-alignment and supply-chain gates on every PR. 5-target CI matrix: Linux x86_64/aarch64, macOS x86_64/arm64, Windows x86_64. |
| **Reference apps** | Web, iOS, macOS, Android, Windows, Linux, and CLI clients live in [`traverse-framework/reference-apps`](https://github.com/traverse-framework/reference-apps). |
| **Latency** | No container runtime; one binary per platform. Measured cold-start and steady-state methodology in [docs/benchmarks.md](docs/benchmarks.md). |

### Toward v1.0.0

v1.0.0 is **gated, not dated** — governed by
[`spec 049-v1-milestone-gate`](specs/049-v1-milestone-gate/spec.md), checkable
with `bash scripts/ci/v1_gate_check.sh`. It signals stable public API surfaces,
every published crate live on crates.io, and the runtime stress-tested on every
supported platform. Full conditions: [docs/v1-milestone.md](docs/v1-milestone.md).

Explicitly **not** required for v1.0.0: a reference app in this repo (that's
`reference-apps`), an HTTP admin API, a cloud deployment surface, or
worker-isolation message passing (v2).

---

## How it works

| Crate | Role |
|---|---|
| `traverse-runtime` | Core execution engine — validates, places, and executes capabilities |
| `traverse-contracts` | Contract definitions, parsing, and validation |
| `traverse-cli-rs` | Command-line interface (binary: `traverse-cli`) — register, list, validate, run |
| `traverse-mcp` | Model Context Protocol stdio server and governed MCP-facing surface |
| `traverse-embedder` | Public Rust embedder SDK (`embedder-api/1.0.0`) for Linux GTK and CLI clients |
| `traverse-expedition-wasm` | Expedition example domain compiled to `wasm32-wasi` |
| `traverse-native-bridge` | Deterministic builder for the governed native runtime WebAssembly bridge |
| `traverse-swift-host` | Apple static-library feasibility host for the Traverse runtime bridge |

`traverse-registry` (capability and event registries with deterministic
traversal) lives in [`traverse-framework/registry`](https://github.com/traverse-framework/registry)
so capabilities publish and version independently of the runtime. The runtime
never talks to that repo live — `traverse-cli registry sync` pulls a signed
index artifact into local workspace state, and execution reads local state
only.

Boundaries and portability rules: [docs/adapter-boundaries.md](docs/adapter-boundaries.md) ·
[docs/compatibility-policy.md](docs/compatibility-policy.md) ·
[docs/decision-log.md](docs/decision-log.md).

---

## For Agents

This project supports AI-assisted development with Codex and Claude Code running
in parallel.

| Agent | File | Purpose |
|---|---|---|
| Claude Code | [`CLAUDE.md`](CLAUDE.md) | Project context, governance rules, speckit workflow |
| Codex | [`AGENTS.md`](AGENTS.md) | Project context, coordination rules, speckit workflow |
| All agents | [`.specify/memory/constitution.md`](.specify/memory/constitution.md) | Governing constitution — mirrors [`traverse-framework/.github`](https://github.com/traverse-framework/.github) at the version in [`.governance-version`](.governance-version) |

Workflow: read your entry-point file → claim the ticket (check `agent:claude` /
`agent:codex` labels and existing branches) → branch `NNN-feature-name` → write
`specs/<branch>/spec.md` before code → implement the smallest change that
satisfies the spec → open a PR with `## Governing Spec`, `## Project Item`, and
`## Validation` sections. Full rules: [`docs/multi-thread-workflow.md`](docs/multi-thread-workflow.md).

---

## Governance

Code must align with an approved, immutable spec or it does not merge.

| Artifact | Location | Role |
|---|---|---|
| Specs | [`specs/`](specs/) | Versioned, immutable, merge-gating |
| Contracts | [`contracts/`](contracts/) | Source of truth for runtime behavior |
| Constitution | [`.specify/memory/constitution.md`](.specify/memory/constitution.md) | Overrides all convenience decisions |
| CI gate | [`scripts/ci/spec_alignment_check.sh`](scripts/ci/spec_alignment_check.sh) | Deterministic, AI-agnostic |

Start with [001-foundation-v0-1](specs/001-foundation-v0-1/spec.md) (core
runtime, CLI, MCP surface), [004-spec-alignment-gate](specs/004-spec-alignment-gate/spec.md)
(the CI gate), and [049-v1-milestone-gate](specs/049-v1-milestone-gate/spec.md)
(what v1.0.0 requires).

[GitHub Project 1](https://github.com/orgs/traverse-framework/projects/1/) is
the canonical board — all active work has an issue, a project item, and a PR.

---

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md),
[SECURITY.md](SECURITY.md), and [docs/quality-standards.md](docs/quality-standards.md)
before opening a PR. Every PR must be backed by an approved spec.

Common starting points: [docs/getting-started.md](docs/getting-started.md) ·
[docs/troubleshooting.md](docs/troubleshooting.md) ·
[docs/what-can-i-build.md](docs/what-can-i-build.md).

---

## Built on UMA

| | UMA | Traverse |
|---|---|---|
| What it is | Architecture model + book | Working runtime implementation |
| Business capabilities | Defines the concept | Executes them with contracts and specs |
| Portability | Describes the pattern | Enforces it through WASM and adapters |
| Governance | Specifies the rules | Immutable specs and CI gates |
| AI safety | Describes requirements | Explainable runtime traces |

Read the [UMA book](https://www.universalmicroservices.com/) and the
[UMA code examples](https://github.com/enricopiovesan/UMA-code-examples).

---

<details>
<summary><strong>Full documentation index</strong></summary>

### Authoring

- [docs/capability-contract-authoring-guide.md](docs/capability-contract-authoring-guide.md) — capability contracts
- [docs/event-contract-authoring-guide.md](docs/event-contract-authoring-guide.md) — event contracts
- [docs/wasm-agent-authoring-guide.md](docs/wasm-agent-authoring-guide.md) — WASM capability authoring
- [docs/wasm-microservice-authoring-guide.md](docs/wasm-microservice-authoring-guide.md) — WASM microservice authoring
- [docs/workflow-composition-guide.md](docs/workflow-composition-guide.md) — composing workflows
- [docs/capability-publish.md](docs/capability-publish.md) — publishing to the registry
- [docs/expedition-example-authoring.md](docs/expedition-example-authoring.md) — worked example

### Releases

- [docs/releases/v0.10.1.md](docs/releases/v0.10.1.md) — current release notes
- [docs/releases/v0.9.1.md](docs/releases/v0.9.1.md) — prior release notes
- [docs/releases/v0.8.1.md](docs/releases/v0.8.1.md) — prior release notes

### Consumer and packaging paths

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
- [docs/benchmarks.md](docs/benchmarks.md) — measured latency methodology
- [docs/decision-log.md](docs/decision-log.md) — consolidated architecture decisions

</details>

---

## License

Apache-2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

## Related Work

- [UMA-code-examples](https://github.com/enricopiovesan/UMA-code-examples)
- [Universal Microservices Architecture — Book](https://www.amazon.com/dp/B0GTTTTQH4)
- [Contract-Driven AI Development (C-DAD) — White Paper](https://drive.google.com/file/d/1HC_ZWJl9aYaMeN78qiL3ZYBVY7mAGl3f/view)
- [Speaking](https://enricopiovesan.github.io/enricopiovesan/)
- [github.com/enricopiovesan](https://github.com/enricopiovesan)

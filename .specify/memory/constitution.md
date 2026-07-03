# Traverse Constitution

> **Governance note**: this repo's constitution is now also mirrored as the shared, org-wide constitution in [`traverse-framework/.github`](https://github.com/traverse-framework/.github) (governance version 1.0.0), so other repos (`registry`, etc.) can adopt the same principles without duplicating them independently. This copy remains authoritative for this repo and satisfies this repo's own CI content checks — update both together if you amend a shared principle.

## Core Principles

### I. Capability-First Boundaries
Every feature MUST be modeled as one or more business capabilities, not as UI flows, transport handlers, CRUD wrappers, storage-first modules, or framework-specific components. A valid capability MUST represent one meaningful business action, define clear inputs and outputs, document side effects, name an owner, and be reusable across more than one workflow. Capabilities that are too small, too technical, or too broad MUST be rejected, split, or reframed before implementation.

### II. Contracts Are the Source of Truth
Every capability and event MUST be defined by an explicit contract before implementation begins. Contracts MUST describe identity, versioning, lifecycle state, inputs, outputs, preconditions, postconditions, side effects, dependencies, ownership, permissions, execution constraints, emitted events, consumed events, and policy-relevant metadata. Code, generated artifacts, and runtime behavior MUST conform to the contract; when code and contract disagree, the contract wins until formally amended. Published contracts SHOULD be treated as immutable records with provenance and validation evidence.

Contracts MUST NOT become publishable by automation alone in `v0.1`. Publication candidates MUST pass required automated validation and MUST also receive explicit manual approval before publication.

### III. Specs Are Versioned, Immutable, and Merge-Gating
Formal specs MUST be versioned artifacts and MUST be treated as immutable sources of truth once approved for implementation. Code generation, manual implementation, and test design MUST align with the approved spec version. Pull requests MUST fail validation when implementation, contracts, tests, or generated artifacts drift from the governing spec. No code change may be merged if it is not validated against the relevant approved spec.

### IV. Portability Over Host Coupling
Business logic MUST remain portable across execution environments and MUST NOT be tightly coupled to a specific app shell, framework, cloud runtime, or infrastructure vendor. Environment-specific concerns MUST be isolated behind explicit runtime interfaces and adapters. For `v0.1`, implementations MAY execute locally only, but the design MUST preserve future portability to browser, edge, cloud, worker, and on-device targets.

### V. Discoverability and Governance by Default
Capabilities and events MUST be discoverable through registries and a queryable metadata model, not only through code knowledge or static wiring. Ownership, lifecycle state, versioning, classification, and other governance metadata MUST be explicit and machine-readable. Event contracts MUST be treated as first-class assets, parallel to capability contracts, with the same expectations for discoverability, governance, and evolution.

### VI. Runtime Decisions Must Be Explainable
Runtime behavior MUST be formed through explicit evaluation of contracts, constraints, policies, and context. Constraints determine validity, policies determine preference, and traces MUST record how a decision was formed, including rejected alternatives and execution failures when relevant. The runtime MUST remain a control plane that interprets contracts consistently rather than a collection of hidden ad hoc decisions embedded inside capabilities.

### VII. Small, Verifiable v0.1
`v0.1` MUST stay focused on the smallest demonstrable slice of the vision: define a capability contract, register it, validate it, discover it, and execute it locally through the runtime with structured trace output. Work that introduces distributed orchestration, advanced placement optimization, full AI planning, full workflow engines, federated registries, or UI-heavy surfaces is out of scope unless the constitution is amended. Every increment MUST be testable end-to-end through CLI-driven flows and contract validation.

## Product Constraints

Traverse is a runtime and composition model for business capabilities, not an application framework.

The initial repository and design work MUST prioritize these core components:

- Capability contract spec
- Event contract spec
- Contract validator
- Capability registry
- Event registry
- Minimal metadata graph
- Runtime core for local execution
- Constraint evaluation
- Structured trace generation
- CLI commands to register, list, validate, and run capabilities

The following are explicit non-goals for `v0.1`:

- Distributed orchestration
- Edge and cloud placement optimization
- Multi-cloud runtime execution
- Full AI planner
- Full workflow engine
- Full graphical UI
- Federated registry mesh

Brownfield support SHOULD focus on extracting existing business logic behind explicit contracts, provenance, and validation evidence. Greenfield support SHOULD focus on defining capabilities and contracts first, then composing them through the graph.

## Non-Functional Requirements

The following non-functional qualities are mandatory for in-scope work:

- Reliability: runtime, registries, and contract validation flows MUST fail predictably and expose actionable error states.
- Determinism: core decision logic, validation, and trace generation MUST behave deterministically for the same inputs and governing artifacts.
- Traceability: runtime behavior, validation outcomes, and merge-gating decisions MUST produce inspectable evidence.
- Portability: capability implementations MUST preserve portability and avoid accidental host coupling.
- Testability: core runtime and business logic MUST be designed for full automated verification.
- Maintainability: architecture, naming, and boundaries MUST support long-term evolution without hidden coupling.
- Security and trust: signed, versioned, validated, and provenance-aware artifacts are the expected direction, and `v0.1` work MUST not block that path.
- Performance: local execution and browser demo behavior MUST remain responsive enough for interactive development and demonstration workflows.
- Reproducibility: builds, tests, and generated artifacts SHOULD be reproducible from pinned inputs, toolchains, and commands.
- Observability: runtime, validation, and merge-gating flows MUST emit structured evidence suitable for diagnosis and audit.
- Compatibility discipline: versioned surfaces MUST define and preserve explicit backward-compatibility rules.

## Non-Negotiable Quality Standards

The following are non-negotiable and must block merge when violated:

- Approved spec alignment
- Contract alignment
- Production-grade code quality
- 100% coverage for core business and runtime logic
- Passing automated validation and test flows
- No silent ambiguity in runtime selection
- No hidden bypass of contract, policy, constraint, or trace mechanisms
- No unreviewed exception to portability or host-coupling rules
- No unreviewed architectural change without a decision record when the change affects core contracts, runtime behavior, registry behavior, versioning, or quality gates

## Enterprise Quality Standards

The following standards are required for enterprise-grade engineering quality:

- Architecture decision records for material technical decisions affecting contracts, runtime, registries, versioning, security, or quality gates
- Dependency policy with pinned versions, minimized dependency footprint, and review of new dependencies for security and licensing impact
- Reproducible build and validation commands documented and used consistently in CI
- Static analysis gates appropriate to the stack, including formatting, linting, and dependency/security checks
- Security baseline with strong input validation, explicit exception handling for unsafe or privileged code paths, and a path toward provenance and artifact trust
- Structured observability with execution identifiers, structured runtime evidence, and actionable error taxonomy
- Compatibility policy for spec versions, contract versions, and runtime + MCP surfaces
- Test taxonomy covering unit, contract, integration, end-to-end, and regression or golden-path validation where applicable
- Documentation standards for public modules, artifacts, failure modes, and examples
- Explicit exception process requiring owner, rationale, and review for any deviation from portability, quality, coverage, or merge-gating standards

## Development Workflow

All substantive work MUST follow this sequence:

1. Clarify the business action and validate that the proposed capability boundary is meaningful.
2. Define or amend the governing versioned spec.
3. Define or amend the relevant capability and event contracts.
4. Define required constraints, policies, and lifecycle assumptions.
5. Specify how the capability will be registered, validated, discovered, and executed.
6. Add or update tests for contract validation, spec alignment, registry behavior, runtime execution, trace output, and graph or event interactions as applicable.
7. Implement the smallest change that satisfies the spec, contract, and `v0.1` scope.

Every spec, plan, and task list MUST explicitly state:

- Which capability or event is being introduced or changed
- Which spec version governs the change
- Why the boundary is correct
- What contract fields are required
- What governance metadata is required
- Which constraints and policies apply
- Whether the change is in or out of `v0.1` scope
- How the behavior will be verified from the CLI or other stable runtime entry points
- What trace or validation evidence should exist after execution

All meaningful work MUST be tracked through:

- a GitHub issue
- a Project 1 item
- a pull request

These three artifacts are the required minimum traceability model unless an approved exception is documented.

## Governance

This constitution overrides convenience-based implementation decisions, hidden coupling, and speculative architecture growth.

All reviews MUST check for:

- Spec/version alignment and drift risk
- Capability boundary quality
- Capability and event contract completeness
- Portability risks
- Hidden host or framework coupling
- Discoverability and governance metadata quality
- Correct separation of constraints, policies, and execution logic
- Adequate verification of registry, contract, runtime, trace, and graph behavior
- Scope creep beyond `v0.1`

Amendments require documenting:

- The rule being changed
- The reason the current rule is insufficient
- The migration or compatibility impact
- The new version of the constitution

**Version**: 1.2.0 | **Ratified**: 2026-03-26 | **Last Amended**: 2026-03-26

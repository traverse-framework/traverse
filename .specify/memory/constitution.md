# Traverse Constitution Pointer

This repo does not keep a copy of the org constitution. The governing constitution for every `traverse-framework` repo, including this one, lives in [`traverse-framework/.github`](https://github.com/traverse-framework/.github/blob/bootstrap-governance/constitution.md): its core principles (specs are versioned, immutable and merge-gating; contracts are the source of truth), Non-Functional Requirements, Enterprise Quality Standards, development workflow and amendment process. Read it before any implementation work. The adopted governance release is in `.governance-version`, the file CI reads.

Only the Traverse-specific rules below live here. They add to the org constitution and never relax it; amend them through a spec or decision-log entry in this repo.

## Traverse-Specific Rules

### Small, Verifiable v0.1
`v0.1` MUST stay focused on the smallest demonstrable slice of the vision: define a capability contract, register it, validate it, discover it, and execute it locally through the runtime with structured trace output. Work that introduces distributed orchestration, advanced placement optimization, full AI planning, full workflow engines, federated registries, or UI-heavy surfaces is out of scope unless the constitution is amended. Every increment MUST be testable end-to-end through CLI-driven flows and contract validation.

### Product Constraints

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

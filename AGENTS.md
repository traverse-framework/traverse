# Traverse Development Guidelines

Auto-generated from all feature plans. Last updated: 2026-04-07

## Active Technologies

- Rust 1.94+
- Cargo workspace
- serde (JSON serialization)
- semver
- WASM (target)

## Project Structure

```text
crates/
  traverse-runtime/      # Core execution engine
  traverse-contracts/    # Contract definitions and validation
  traverse-registry/     # Capability and event registries
  traverse-cli/          # Command-line interface
  traverse-mcp/          # Model Context Protocol (stub)
specs/                   # Versioned, immutable governing specs
contracts/               # Capability and event contracts
docs/                    # ADRs, quality standards, policies
.specify/                # Speckit: constitution, scripts, templates
scripts/ci/              # Deterministic spec-alignment gate
```

## Commands

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo run -p traverse-cli
bash scripts/ci/spec_alignment_check.sh
```

## Code Style

- No `unsafe`, no `unwrap()`, no `panic!()`, no TODO comments
- 100% test coverage for core business and runtime logic
- Deterministic: same inputs must produce same outputs

## Lean Implementation

Before adding code, apply the Traverse minimality ladder:

1. Confirm the change is required by the active issue and governing spec.
2. Reuse existing Traverse code, contracts, specs, and docs when they already fit.
3. Prefer the Rust standard library, Cargo workspace, or existing dependencies over new abstractions.
4. Prefer a schema, validation branch, test, or documentation update when that solves the issue.
5. Prefer one focused function, CLI branch, or manifest field before adding broader structure.
6. Add only the minimum new structure needed for the issue.

Minimality must not reduce spec alignment, contract validation, stable error codes,
security, traceability, accessibility, or required tests.

## Recent Changes

- 188-codex-agent-coordination: Added Codex claim check and AGENTS.md coordination rules

<!-- MANUAL ADDITIONS START -->
## Agent Coordination

**Before starting any work on an issue**, run these pre-flight checks:

### 1. Check for Claude Code claim

```bash
gh issue view <NUMBER> --repo traverse-framework/Traverse --json labels
```

If the labels include `agent:claude` → **STOP**. Report:
> Issue #\<NUMBER\> is claimed by Claude Code. Choose a different ticket.

### 2. Check for Claude Code branch

```bash
git ls-remote --heads origin | grep "issue-<NUMBER>-"
```

If a `claude/issue-<NUMBER>-*` branch exists → **STOP**. Report:
> A Claude Code branch already exists for issue #\<NUMBER\>. Choose a different ticket.

### 3. Claim the ticket (only if pre-flight passes)

```bash
# Add label
gh issue edit <NUMBER> --repo traverse-framework/Traverse --add-label "agent:codex"

# Get project item ID with bounded output
gh project item-list 1 --owner traverse-framework --format json --limit 300 \
  --jq '.items[] | select(.content.number == <NUMBER>) | .id'

# Set Agent → Codex
gh project item-edit --project-id PVT_kwHOAEZXvs4BS6Ns \
  --id <ITEM_ID> \
  --field-id PVTSSF_lAHOAEZXvs4BS6NszhBK-Qk \
  --single-select-option-id 34d6db7d

# Set Status → In Progress
gh project item-edit --project-id PVT_kwHOAEZXvs4BS6Ns \
  --id <ITEM_ID> \
  --field-id PVTSSF_lAHOAEZXvs4BS6NszhATmdM \
  --single-select-option-id 47fc9ee4
```

### 4. Governance

Read `.specify/memory/constitution.md` before any implementation work.

- **Spec-first**: every feature needs an approved spec in `specs/` before code
- **Contract-first**: contracts are source of truth; code conforms to contracts
- **Spec-alignment gate**: CI blocks PRs that drift from `specs/governance/approved-specs.json`
- **Traceability**: all work must have a GitHub issue + Project 1 item + PR
<!-- MANUAL ADDITIONS END -->

# Feature Specification: Rewrite README for best-in-class open source

**Feature Branch**: `190-readme-rewrite`
**Created**: 2026-04-07
**Status**: Approved (retroactively, 2026-07-18 — see decision-log.md Decision 26; implementation independently verified complete in README.md)
**Input**: GitHub issue #190

> **Governance note**: README.md is governed by spec `001-foundation-v0-1`. This spec must be declared in the PR body. No runtime, contract, or CI gate changes.

## User Scenarios & Testing

### User Story 1 - Human contributor lands on the repo and immediately understands the project (Priority: P1)

**Acceptance Scenarios**:
1. **Given** the README, **When** a developer reads it, **Then** they understand the purpose, can build and run in under 5 minutes, and know how to contribute.
2. **Given** the README, **When** a visitor scans it, **Then** badges communicate CI status, coverage, license, and maturity at a glance.

### User Story 2 - AI agent loads project context from the README (Priority: P2)

**Acceptance Scenarios**:
1. **Given** the README, **When** an AI agent starts a session, **Then** it knows where to find CLAUDE.md, AGENTS.md, the constitution, and the speckit workflow.
2. **Given** the README, **When** Codex reads AGENTS.md, **Then** the agent path is consistent with the README's agent section.

### User Story 3 - Repo discovery is optimized (Priority: P3)

**Acceptance Scenarios**:
1. **Given** GitHub repo metadata, **When** someone searches for portable Rust runtimes or MCP, **Then** Traverse appears with a clear description and relevant topics.

## Requirements

- **FR-001**: README MUST have a hero section with CI, coverage, license, Rust version, and spec-governed badges.
- **FR-002**: README MUST have a human quick-start section (build, test, run).
- **FR-003**: README MUST have an agent entry point section referencing CLAUDE.md, AGENTS.md, and the constitution.
- **FR-004**: GitHub repo MUST have an updated description, topic tags, and homepage URL.

## Success Criteria

- **SC-001**: All CI badges resolve to live workflow runs.
- **SC-002**: A new contributor can build and run the CLI from the README alone.
- **SC-003**: An AI agent session started from the README can locate all governance artifacts without searching.
- **SC-004**: All CI checks pass.

## Assumptions

- The existing screenshot asset at the top of the README can be replaced or removed in favor of the badge row.
- `001-foundation-v0-1` governs README.md and must be declared in the PR.

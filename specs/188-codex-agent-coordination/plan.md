# Implementation Plan: Configure Codex to skip tickets claimed by Claude Code

**Branch**: `188-codex-agent-coordination` | **Date**: 2026-04-07 | **Spec**: [spec.md](spec.md)

## Summary

Update the Codex dev thread starter prompt and AGENTS.md to include a pre-flight claim check (skip if `agent:claude` label or `claude/issue-NNN-*` branch exists) and a claim step (add `agent:codex` + set Agent → Codex on Project 1). Update `docs/multi-thread-workflow.md` to document the two-agent model. No Rust code, no CI gate, no contract changes.

## Technical Context

**Language/Version**: Bash (gh CLI), Markdown
**Primary Dependencies**: `gh` CLI, GitHub Labels API, GitHub Projects API
**Storage**: N/A
**Testing**: Manual — start Codex on a claimed issue, verify it stops
**Target Platform**: Developer workstation; Codex CLI
**Project Type**: Dev tooling — prompt and instructions update
**Constraints**: Must not break existing single-agent Codex workflows; AGENTS.md manual additions must be preserved

## Constitution Check

Dev tooling only. All gates pass or N/A. No governed files touched.

## Project Structure

### Documentation (this feature)

```text
specs/188-codex-agent-coordination/
├── spec.md
└── plan.md              # This file
```

### Files touched

```text
AGENTS.md                             # CREATED — Codex entry point with coordination rules
docs/multi-thread-workflow.md         # MODIFIED — two-agent model + updated starter prompts
```

## Phase 0: Research

Resolved:
- Codex is human-triggered via starter prompt in `docs/multi-thread-workflow.md` (lines 94–104) — no automated queue scanning
- AGENTS.md does not yet exist; `update-agent-context.sh` generates it but manual additions between markers are preserved
- Project IDs: Agent field `PVTSSF_lAHOAEZXvs4BS6NszhBK-Qk`, Codex option `34d6db7d`; Status field `PVTSSF_lAHOAEZXvs4BS6NszhATmdM`, In Progress `47fc9ee4`; Project `PVT_kwHOAEZXvs4BS6Ns`

## Phase 1: Design

### Dev thread starter prompt additions

Two steps prepended to the existing prompt:

**Pre-flight (runs before any work):**
```
Before starting any work on issue #<NUMBER>:
1. gh issue view <NUMBER> --repo traverse-framework/Traverse --json labels
   If labels include "agent:claude" → STOP. Report: "Issue #<NUMBER> is claimed by Claude Code."
2. git ls-remote --heads origin | grep "issue-<NUMBER>-"
   If a claude/issue-<NUMBER>-* branch exists → STOP. Report: "A Claude Code branch exists for #<NUMBER>."
```

**Claim step (runs only if pre-flight passes):**
```
1. gh issue edit <NUMBER> --repo traverse-framework/Traverse --add-label "agent:codex"
2. Retrieve project item ID for issue #<NUMBER> from Project 1, then:
   gh project item-edit --project-id PVT_kwHOAEZXvs4BS6Ns --id <ITEM_ID> \
     --field-id PVTSSF_lAHOAEZXvs4BS6NszhBK-Qk --single-select-option-id 34d6db7d
   gh project item-edit --project-id PVT_kwHOAEZXvs4BS6Ns --id <ITEM_ID> \
     --field-id PVTSSF_lAHOAEZXvs4BS6NszhATmdM --single-select-option-id 47fc9ee4
```

### AGENTS.md structure

Same structure as CLAUDE.md — auto-update markers with a `## Agent Coordination` section in the manual additions block so Codex always sees the rules regardless of how it is started.

## Implementation Sequence

1. Create `AGENTS.md` with coordination rules in the manual additions block
2. Update `docs/multi-thread-workflow.md` — dev thread starter prompt + two-agent section
3. Verify `cargo test` unchanged
4. Commit, push, open PR referencing #188

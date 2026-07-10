---
name: "traverse-ops"
description: "Start or resume the standard Traverse operating model when the user says TRAVERSE OPS, asks to start Traverse ops/dev work, asks for the ready-ticket worker, PR finisher, or backlog gardener, or wants Codex to pick ready Project 1 work and run the Traverse coordination process."
---

# Traverse Ops

Use this skill when the user wants Codex to start or resume the standard Traverse operating model.

Canonical trigger:

```text
TRAVERSE OPS
```

## Workflow

1. Read `AGENTS.md` and follow the agent coordination rules.
2. Read the constitution (via `traverse-framework/.github`, pinned in `.governance-version`) only when the ticket touches architecture, contracts, or versioned surfaces — lazy-read map in the org's `docs/ai-agent-hardening.md`.
3. Inspect current GitHub and Project 1 state.
4. Prefer finishing existing open PRs before claiming new Ready work.
5. Keep cycling through open PRs and Ready Project 1 issues until no actionable
   Ready tickets remain. Do not stop after one PR, one issue, or one merge.
6. If no active PR needs attention, pick the next Ready Project 1 issue.
7. Before work on an issue, run the Claude pre-flight checks from `AGENTS.md`:
   - issue must not have `agent:claude`
   - no remote `claude/issue-NNN-*` branch may exist
8. If pre-flight passes, claim the issue:
   - add `agent:codex`
   - set Project 1 `Agent` to `Codex`
   - set Project 1 `Status` to `In Progress`
9. Use a dedicated `codex/issue-NNN-*` branch.
10. Keep work scoped to the claimed issue and governing spec.
11. Open a dedicated PR using the org body superset (`## Summary`, `## Governing Spec`, `## Project Item`, `## Definition of Done`, `## Validation`) with validation evidence, then immediately queue it: `gh pr merge <N> --squash --auto`. Do not poll checks — continue the loop and release on a later pass once merged. This repo requires branches up to date with `main` (strict checks): rebase when behind, or auto-merge cannot fire.
12. The stop condition is strict: no mergeable or fixable open PRs, no Ready
    Project 1 tickets that pass pre-flight, no uncompleted `agent:codex`
    tickets, and only explicitly blocked work remains. If a Ready ticket is
    blocked, update its ticket/Project state with the concrete blocker before
    continuing to the next Ready ticket.

## Gates & Failure Playbook

Every PR must pass the org gates `cla / cla` and `baseline / governance-baseline` plus this repo's CI. When a governance gate fails, don't debug from scratch — use the failure playbook in `traverse-framework/.github` `docs/runbook.md` (CLA `recheck` comment; re-runs pin stale gate snapshots, push a commit instead; secret-visibility check). Dependabot PRs get their bodies auto-filled by the org `dependabot-hygiene` workflow — never hand-write them; for one that predates it, comment `@dependabot rebase`, queue `gh pr merge --squash --auto`, and let CI decide.

## Token Discipline

Org-canon token rules live in `traverse-framework/.github` `docs/ai-agent-hardening.md`
(pinned via `.governance-version`): bounded `--limit` queries with server-side `--jq`,
no raw board/CI/test log dumps, targeted diffs before large ones, short progress
updates. Traverse-specific additions:

- After CI starts, poll with bounded output; on failure fetch only that job's log
  and extract the actionable lines.
- Prefer local reproduction (`cargo test`, `bash scripts/ci/spec_alignment_check.sh`)
  before fetching large remote logs.
- Final updates: merged PRs, validations, next recommended issue — not every
  command output.

## Minimality Ladder

Before adding code, apply this Traverse-specific minimality ladder:

1. Does this change need to exist for the active issue and governing spec?
2. Can existing Traverse code, contracts, specs, or docs already satisfy it?
3. Can the Rust standard library, Cargo workspace, or an existing dependency do it?
4. Can a schema, validation branch, test, or documentation update solve it without
   a new abstraction?
5. Can one focused function, CLI branch, or manifest field solve it?
6. Only then add the minimum new structure needed for the issue.

Minimality must never weaken spec alignment, contract validation, stable error
codes, security, traceability, accessibility, or required tests. Create follow-up
tickets for useful adjacent improvements instead of expanding an active slice.

## Operating Lanes

- **Ready-ticket worker**: claim one Ready Project 1 issue and implement it end to end.
- **PR finisher**: inspect open PRs, fix CI/review issues, update stale branches, and merge when green if allowed.
- **Backlog gardener**: audit Project 1 statuses, labels, blockers, notes, and missing tickets.

## Stop Condition

Traverse ops is a drain loop, not a single-ticket worker. Continue alternating
between the PR finisher and Ready-ticket worker lanes until Project 1 has no
actionable Ready work left.

Only stop when one of these is true:

- all open PRs are merged, blocked by an external dependency, or no longer
  actionable for Codex;
- every Ready Project 1 ticket has either been completed or moved out of Ready
  with a concrete blocker recorded;
- a required external permission, token, network/API quota, or user decision
  prevents both PR finishing and Ready-ticket progress.

When stopping for a blocker, report the exact blocked PR or ticket, what was
verified, and what external action would unblock the next pass.

## Guardrails

- Do not mark work `In Progress` unless a real dev thread has started it.
- Do not use labels as status; Project 1 status is the actionability source of truth.
- Do not claim work already owned by Claude Code.
- Do not broaden scope beyond the issue and governing spec.
- Create future tickets for non-blocking improvements instead of expanding an active slice.

For the full operating model, see `docs/multi-thread-workflow.md`.

---
name: traverse-ops
description: Start or resume the standard Traverse operating model when the user says TRAVERSE OPS, asks to start Traverse ops/dev work, asks for the ready-ticket worker, PR finisher, or backlog gardener, or wants the agent to pick ready Project 1 work and run the Traverse coordination process. Execution only — picks up existing Ready tickets; not for planning, brainstorming, or spec creation.
---

<!-- GENERATED from .agents/skills by scripts/agent/sync_skills.sh. Edit the source, then re-run the script. -->

# Traverse Ops

Use this skill when the user wants this agent (Claude Code, Codex or Cursor) to start or resume the standard Traverse operating model. It is the single source for every tool; `scripts/agent/sync_skills.sh` generates the Claude Code and Cursor copies.

Canonical trigger:

```text
TRAVERSE OPS
```

This skill is **execution-only**. It drives already-Ready Project 1 tickets
through implementation to a merged PR. It is not the entry point for planning,
brainstorming new features, defining new specs, or creating new tickets;
handle those as a normal conversation.

## Workflow

1. Read `AGENTS.md` and follow the agent coordination rules.
2. Read the constitution (via `traverse-framework/.github`, pinned in `.governance-version`) only when the ticket touches architecture, contracts, or versioned surfaces — lazy-read map in the org's `docs/ai-agent-hardening.md`.
3. Inspect current GitHub and Project 1 state.
4. Prefer finishing existing open PRs before claiming new Ready work.
5. Keep cycling through open PRs and Ready Project 1 issues until no actionable
   Ready tickets remain. Do not stop after one PR, one issue, or one merge.
6. If no active PR needs attention, pick the next Ready Project 1 issue.
7. Before work on an issue, run `scripts/agent/claim.sh <N> <tool> --dry-run`
   (`<tool>` is `claude`, `codex` or `cursor`). It checks that the issue is
   Ready, carries no `agent:*` label, and has no `*/issue-<N>-*` branch or open
   PR from any tool. If it refuses, pick another ticket.
8. Claim with `scripts/agent/claim.sh <N> <tool>`. That adds `agent:<tool>`,
   sets Project 1 to In Progress, and posts the audit comment.
9. Use the branch it prints (`<tool>/issue-<N>-*`).
10. Keep work scoped to the claimed issue and governing spec.
11. Open a dedicated PR using the org body superset (`## Summary`, `## Governing Spec`, `## Project Item`, `## Definition of Done`, `## Validation`) with validation evidence. For full-CI PRs follow **Merge Gate** (add `needs-review`, wait for a passing review record) before squash-merge. Continue the drain loop while CI/review runs. This repo requires branches up to date with `main`: rebase when behind.
12. The stop condition is strict: no mergeable or fixable open PRs, no Ready
    Project 1 tickets that pass pre-flight, no uncompleted tickets carrying
    this tool's `agent:*` label, and only explicitly blocked work remains. If a Ready ticket is
    blocked, update its ticket/Project state with the concrete blocker before
    continuing to the next Ready ticket.

### Continuation Rule (mandatory)

Do **not** end a turn after claiming a ticket, opening/queuing/rebasing a PR,
posting a status update, or completing one ticket. Each of those is an
intermediate state change, not a completion boundary. In the same active
Traverse Ops run, immediately resume at workflow step 3 and take the next
actionable operation: finish an open PR, reconcile a merged ticket, or claim
and execute the next eligible Ready ticket.

Only send a final response when the strict stop condition above has been
verified in that same run. The final response must state which stop condition
was met. If execution time or context is constrained, continue with the
smallest next concrete state change rather than returning a progress-only
message.

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
- **Review queue**: drain `needs-review` PRs from other tools per Decision 133.

## Stop Condition

Traverse ops is a drain loop, not a single-ticket worker. Continue alternating
between the PR finisher and Ready-ticket worker lanes until Project 1 has no
actionable Ready work left.

Only stop when one of these is true:

- all open PRs are merged, blocked by an external dependency, or no longer
  actionable for this tool;
- every Ready Project 1 ticket has either been completed or moved out of Ready
  with a concrete blocker recorded;
- a required external permission, token, network/API quota, or user decision
  prevents both PR finishing and Ready-ticket progress.

When stopping for a blocker, report the exact blocked PR or ticket, what was
verified, and what external action would unblock the next pass.

## Review Queue (Decision 133)

Before claiming new Ready work, drain the cross-tool review queue:

1. List open PRs labelled `needs-review` whose head branch prefix is a **different**
   tool than this session (`claude/`, `codex/`, or `cursor/`).
2. Claim one (comment that this tool is reviewing; do not review your own prefix
   except via the 4-hour fallback below).
3. In a **fresh** context, review the PR head against the rubric in
   `traverse-framework/.github` `docs/ai-agent-hardening.md` (governing-spec match,
   explicit failure modes, no weakened gates, minimality, behavior-asserting tests).
4. Post a single PR comment whose body starts with the HTML review-record header
   (`tool`, `commit` = current head SHA, `verdict` = `pass` or `changes-requested`,
   `fallback`).
5. On `changes-requested`: wait for the author to fix or rebut, then re-check the
   new head. After one unresolved round, add `needs-enrico`.

Same-tool fallback: only if `needs-review` has been on the PR for ≥ 4 hours, and
only in a fresh session. Record `fallback: true`. Never review your own branch
prefix without that recorded fallback.

## Merge Gate (Decision 133)

For a PR this tool authored that the change classifier sends to full CI:

1. When CI is green, add the `needs-review` label if it is missing.
2. Do **not** merge until a passing review record exists for the current head
   commit from a different tool (or a recorded same-tool fallback after 4 hours).
3. In governance 1.3.x the CI check warns when the record is missing; still treat
   the record as required for merge in this skill.
4. Dependabot and version-only release PRs are exempt; docs-only PRs are exempt
   via `pr_change_classification.sh`.
5. Then squash-merge, run `scripts/agent/release.sh <N> Done`, and continue the drain loop.

Never use `--no-verify` or redirect `core.hooksPath`. First push: write the PR
body file and push with `TRAVERSE_PR_BODY=/path/to/body.md git push -u origin HEAD`
(or `.git/traverse-pr-body.md`). When a new failure costs a full CI round-trip,
add a symptom/cause/fix entry to `docs/agent-pitfalls.md` in the hardening PR.

## Guardrails

- Do not mark work `In Progress` unless a real dev thread has started it.
- Do not use labels as status; Project 1 status is the actionability source of truth.
- Do not claim work another tool owns (`claim.sh` refuses it).
- Do not broaden scope beyond the issue and governing spec.
- Create future tickets for non-blocking improvements instead of expanding an active slice.

For the full operating model, see `docs/multi-thread-workflow.md`.

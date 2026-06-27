# Multi-Thread Workflow

Traverse can support parallel execution, but only when parallel work is real.

One Codex thread is one active worker. If we want true parallel work, we should run multiple Codex threads, each with a separate issue, branch, and pull request.

## Two-Agent Model (Codex + Claude Code)

Codex and Claude Code can work in parallel on separate issues. To prevent conflicts:

- **Labels**: `agent:codex` and `agent:claude` mark which agent owns an issue
- **Project board**: the Agent field (Unassigned / Codex / Claude Code) shows ownership at a glance
- **Branches**: `codex/issue-NNN-*` and `claude/issue-NNN-*` naming makes branch ownership explicit

**Rule**: claim before you code. Both agents check for the other's label and branch before starting work. If already claimed, stop and pick a different ticket.

Claude Code uses the `claim-ticket` skill (`.agents/skills/claim-ticket/SKILL.md`).
Codex uses the pre-flight steps embedded in the dev thread starter prompt below.

## Thread Roles

### PM Thread

The PM thread:

- keeps the backlog, labels, blockers, and Project 1 current
- talks with Enrico about product and governance decisions
- decides when work is `ready`, `blocked`, `in-progress`, or `future`
- does not mark a ticket `in-progress` unless a real worker has started

### Dev Threads

Each dev thread:

- owns exactly one active issue at a time
- works on exactly one `codex/...` branch at a time
- opens exactly one PR for that slice
- updates the issue and PR with validation evidence

Recommended rule:

- one dev thread per issue
- if two issues touch the same files heavily, do not start them in parallel

### Review / Integration Thread

The review thread:

- checks spec alignment
- checks contract and workflow drift
- checks merge conflicts and integration risk
- ensures must-fix findings are fixed in the active PR
- turns non-blocking follow-up work into `future` tickets

## Status Rules

Use statuses this way:

- `Ready`: approved and available to start
- `In Progress`: a real dev thread is actively working the ticket now
- `Blocked`: the ticket cannot continue and the blocker is visible in both the issue body and Project 1 note
- `Future`: valid work that is tracked but intentionally not active now

Do not move work to `In Progress` merely because it is a candidate for parallel execution.

If a ticket has an open PR, it must be labeled `in-progress` and its Project 1 item must also be `In Progress`. The PM thread should fix mismatches immediately.

The backlog audit logic lives in [scripts/ci/project_board_audit.sh](../scripts/ci/project_board_audit.sh).

## Required Parallel Work Rules

For parallel work to be valid:

- each active issue must have a dedicated dev thread
- each active issue must have its own branch
- each active issue must have its own PR
- Project 1 `Status` must match reality
- Project 1 `Note` should identify the worker, branch, or workstream when useful
- run `bash scripts/ci/project_board_audit.sh` after board changes to catch drift early

## Recommended Current Split

For the current expedition artifact work, the cleanest split is:

- Workstream 1: [#42](https://github.com/traverse-framework/Traverse/issues/42) event contracts
- Workstream 2: [#44](https://github.com/traverse-framework/Traverse/issues/44) atomic capability contracts
- Workstream 3: [#43](https://github.com/traverse-framework/Traverse/issues/43) composed capability contract
- Workstream 4: [#45](https://github.com/traverse-framework/Traverse/issues/45) workflow artifact

If we only have one active dev thread, these should remain `Ready`.

If we have four active dev threads, then all four can honestly be `In Progress`.

## Starter Prompts

## Chat Trigger

Use this short trigger when you want Codex to start or resume the standard Traverse operating model without retyping the full instructions:

```text
TRAVERSE OPS
```

When Enrico says `TRAVERSE OPS`, Codex should treat it as:

- Start or resume the ready-ticket worker: pick one Ready Project 1 ticket, follow `AGENTS.md`, claim it, set Project status to `In Progress`, implement one issue on one branch, open one PR, and keep going until merged or genuinely blocked.
- Start or resume the PR finisher: inspect open PRs, rebase/update immediately when behind, fix CI/review issues, rerun gates, merge green PRs, and update linked issues and Project 1 state.
- Start or resume the PM/PO backlog gardener: audit Project 1 statuses, labels, blockers, and notes; ensure Todo items become `Ready` or `Blocked`; ensure Blocked items have a note; create missing tickets with full Definition of Done.
- Do all feasible work autonomously and ask only when a product decision is truly required.
- Do not use labels as status; Project 1 status is the actionability source of truth.
- Run lean by default: use filtered Project 1 queries, bounded command output,
  focused diffs, and summarized CI/test results. Quote only actionable failure
  lines, not full logs or full board JSON.

Use this PM thread prompt:

```text
Act as the Traverse PM / scrum master thread.
Your job is to keep GitHub issues, Project 1, labels, blockers, notes, and PR flow accurate.
Do not mark a ticket in progress unless a real dev thread has started it.
When a problem is must-fix for the active slice, it must be fixed in the active PR.
When a problem is non-blocking, create a future ticket.
Keep all work aligned to approved specs and project-management rules.
```

Use this dev thread prompt:

```text
Act as a Traverse dev thread for issue #NN.

Pre-flight (run before any work):
1. gh issue view NN --repo traverse-framework/Traverse --json labels
   If labels include "agent:claude" → STOP. Report: "Issue #NN is claimed by Claude Code."
2. git ls-remote --heads origin | grep "issue-NN-"
   If a claude/issue-NN-* branch exists → STOP. Report: "A Claude Code branch exists for #NN."

Claim (only if pre-flight passes):
1. gh issue edit NN --repo traverse-framework/Traverse --add-label "agent:codex"
2. Set Agent → Codex and Status → In Progress on Project 1 for this issue.
   Project ID: PVT_kwHOAEZXvs4BS6Ns
   Agent field: PVTSSF_lAHOAEZXvs4BS6NszhBK-Qk, Codex option: 34d6db7d
   Status field: PVTSSF_lAHOAEZXvs4BS6NszhATmdM, In Progress option: 47fc9ee4

Then proceed:
Only work on this issue.
Use a dedicated codex/issue-NN-* branch and open a dedicated PR.
Keep implementation strictly aligned with the governing spec.
If you find a must-fix issue for this slice, fix it in the same PR.
If you find a non-blocking improvement, create or request a future ticket instead of expanding scope.
Do not change ticket or project status unless the PM thread asks for it.
```

Use this review thread prompt:

```text
Act as the Traverse review / integration thread.
Your job is to review active PRs for spec alignment, contract drift, workflow drift, missing tests, merge risk, and governance gaps.
Must-fix findings should stay in the active PR.
Nice-to-have follow-ups should become future tickets.
Keep the repo and board consistent with the approved process.
```

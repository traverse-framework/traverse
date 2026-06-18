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

1. Read `.specify/memory/constitution.md` before implementation work.
2. Read `AGENTS.md` and follow the agent coordination rules.
3. Inspect current GitHub and Project 1 state.
4. Prefer finishing existing open PRs before claiming new Ready work.
5. If no active PR needs attention, pick one Ready Project 1 issue.
6. Before work on an issue, run the Claude pre-flight checks from `AGENTS.md`:
   - issue must not have `agent:claude`
   - no remote `claude/issue-NNN-*` branch may exist
7. If pre-flight passes, claim the issue:
   - add `agent:codex`
   - set Project 1 `Agent` to `Codex`
   - set Project 1 `Status` to `In Progress`
8. Use a dedicated `codex/issue-NNN-*` branch.
9. Keep work scoped to the claimed issue and governing spec.
10. Open a dedicated PR with validation evidence.

## Token Discipline

Use a lean-by-default operating style so long-running Traverse ops sessions do
not waste context on raw logs.

- Prefer targeted GitHub queries over full board dumps. For Ready work, use
  `gh project item-list 1 --owner enricopiovesan --format json --limit 300 --jq '...'`
  and return only issue number, title, labels, and item id.
- Do not paste full `gh project item-list`, `gh pr checks --watch`, test, clippy,
  coverage, or CI logs into the conversation. Summarize pass/fail counts and
  only quote the failing lines needed to fix the issue.
- Use `git diff --stat`, `git diff --name-only`, and focused file hunks before
  large diffs. Open exact line ranges only when a decision depends on them.
- Use `rg` with narrow patterns before broad recursive reads. Avoid reading
  generated files, target directories, lockfile-scale artifacts, and full specs
  unless the active issue requires them.
- Keep progress updates short: current action, discovered blocker if any, and
  next action. Avoid repeating unchanged state.
- After CI starts, poll with bounded output. If checks are pending, report only
  changed status; if a job fails, fetch that job log and extract the actionable
  failure.
- Prefer local reproduction of a failing gate before fetching large remote logs.
- In final updates, include merged PRs, validations, and next recommended issue;
  do not restate every command output.

## Operating Lanes

- **Ready-ticket worker**: claim one Ready Project 1 issue and implement it end to end.
- **PR finisher**: inspect open PRs, fix CI/review issues, update stale branches, and merge when green if allowed.
- **Backlog gardener**: audit Project 1 statuses, labels, blockers, notes, and missing tickets.

## Guardrails

- Do not mark work `In Progress` unless a real dev thread has started it.
- Do not use labels as status; Project 1 status is the actionability source of truth.
- Do not claim work already owned by Claude Code.
- Do not broaden scope beyond the issue and governing spec.
- Create future tickets for non-blocking improvements instead of expanding an active slice.

For the full operating model, see `docs/multi-thread-workflow.md`.

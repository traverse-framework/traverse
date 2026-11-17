---
name: traverse-ops
description: Start or resume the standard Traverse operating model when the user says TRAVERSE OPS, asks to start Traverse ops/dev work, asks for the ready-ticket worker, PR finisher, or backlog gardener, or wants Claude Code to pick ready Project 1 work and run the Traverse coordination process. Execution only — picks up existing Ready tickets; not for planning, brainstorming, or spec creation.
---

# Traverse Ops (Claude Code)

Use this skill when the user wants Claude Code to start or resume the standard
Traverse operating model.

Canonical trigger:

```text
TRAVERSE OPS
```

This skill is **execution-only**: it drives already-Ready Project 1 tickets
through implementation to a PR. It is not the entry point for planning,
brainstorming new features, defining new specs, or creating new tickets —
handle those as a normal conversation.

## Workflow

1. Read the constitution in `traverse-framework/.github` (governance version
   1.0.0) before implementation work.
2. Read `AGENTS.md` and follow the agent coordination rules.
3. Inspect current GitHub and Project 1 state.
4. Prefer finishing existing open PRs before claiming new Ready work.
5. If no active PR needs attention, pick one Ready Project 1 issue.
6. Before work on an issue, run the Codex pre-flight checks:
   - issue must not have `agent:codex`
   - no remote `codex/issue-NNN-*` branch may exist
7. If pre-flight passes, claim the issue:
   - add `agent:claude`
   - set Project 1 `Status` to `In Progress`
8. Use a dedicated `claude/issue-NNN-*` branch.
9. Keep work scoped to the claimed issue and governing spec.
10. Open a dedicated PR with validation evidence.

## Pre-flight and Claim Commands

```bash
# 1. Check for Codex claim
gh issue view <NUMBER> --repo traverse-framework/Traverse --json labels
# If labels include agent:codex → STOP; choose a different ticket.

# 2. Check for Codex branch
git ls-remote --heads origin | grep "issue-<NUMBER>-"
# If a codex/issue-<NUMBER>-* branch exists → STOP; choose a different ticket.

# 3. Claim (only if pre-flight passes)
gh issue edit <NUMBER> --repo traverse-framework/Traverse --add-label "agent:claude"

# Get project item ID with bounded output
gh project item-list 1 --owner traverse-framework --format json --limit 300 \
  --jq '.items[] | select(.content.number == <NUMBER>) | .id'

# Set Status → In Progress (org Project 1; it has no Agent field —
# the agent:claude label is the ownership marker)
gh project item-edit --project-id PVT_kwDOEbiBt84Bbyp1 \
  --id <ITEM_ID> \
  --field-id PVTSSF_lADOEbiBt84Bbyp1zhWglqM \
  --single-select-option-id 47fc9ee4
```

Status option IDs for reference: Ready `f75ad846`, In Progress `47fc9ee4`,
Done `98236657`, Blocked `294b89f5`.

## Token Discipline

Use a lean-by-default operating style so long-running Traverse ops sessions do
not waste context on raw logs.

- Prefer targeted GitHub queries over full board dumps. For Ready work, use
  `gh project item-list 1 --owner traverse-framework --format json --limit 300 --jq '...'`
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


## Continuous Operation

Keep draining the whole Ready backlog in one session — do not stop after one
ticket. When a PR goes CI-green: follow **Merge Gate** (add `needs-review`, wait for record), then merge, mark Done,
and immediately claim the next Ready ticket or fix the next open PR. A PR still
running CI never blocks starting the next ticket. Only stop when no Ready issue
is unclaimed AND every open Claude PR is merged or genuinely blocked.

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
5. Then squash-merge, set Project 1 to Done, and continue the drain loop.

Never use `--no-verify` or redirect `core.hooksPath`. First push: write the PR
body file and push with `TRAVERSE_PR_BODY=/path/to/body.md git push -u origin HEAD`
(or `.git/traverse-pr-body.md`). When a new failure costs a full CI round-trip,
add a symptom/cause/fix entry to `docs/agent-pitfalls.md` in the hardening PR.

## Guardrails

- Do not mark work `In Progress` unless a real dev thread has started it.
- Do not use labels as status; Project 1 status is the actionability source of truth.
- Do not claim work already owned by Codex.
- Do not broaden scope beyond the issue and governing spec.
- Create future tickets for non-blocking improvements instead of expanding an active slice.

For the full operating model, see `docs/multi-thread-workflow.md`.

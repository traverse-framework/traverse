# Project Management

Traverse uses this GitHub Project as the canonical task board:

- [GitHub Project 1](https://github.com/orgs/traverse-framework/projects/1/)

## Working Rule

All meaningful work must be traceable through all three of these artifacts:

- a GitHub issue
- a Project 1 item
- a pull request

This is the default Traverse operating rule for spec slices, implementation slices, governance work, and material documentation changes.

Ticket quality rules are defined in:

- [docs/ticket-standard.md](ticket-standard.md)
- [docs/multi-thread-workflow.md](multi-thread-workflow.md)

## Preferred Flow

1. Start from the governing spec or approved design discussion.
2. Create or link the GitHub issue.
3. Ensure the issue is represented on Project 1.
4. Open a pull request that links the issue or project item.
5. Keep implementation, contracts, and tests aligned with the governing spec.

## Issue Guidance

Issues should describe:

- problem or goal
- affected spec or capability/workflow area
- expected outcome
- any compatibility or governance concerns
- explicit definition of done
- explicit validation steps
- explicit blocker note when blocked

## Pull Request Guidance

Pull requests should include:

- linked issue or project item
- governing spec version
- contract changes, if any
- validation evidence
- ADR reference, if required

Implementation and spec pull requests must declare their governing specs in the PR body under a `## Governing Spec` section. Those declarations are validated against:

- `specs/governance/approved-specs.json`

## Required Traceability

The expected day-to-day rule is:

- one issue per meaningful slice of work
- that issue represented on Project 1
- one pull request implementing or codifying that slice

Exceptions should be rare and should be called out explicitly in the PR notes.

## Board Discipline

Recommended workflow labels:

- `in-progress`
- `blocked`
- `needs-spec`
- `needs-enrico`
- `future`
- `no-spec-needed`

Recommended categories for task tracking:

- specs and architecture
- runtime and contracts
- registries and workflows
- capabilities and examples
- browser demo
- quality and CI

The exact board columns can evolve, but the project board should remain the primary planning surface and the issue should remain the durable record of intent.

Status intent should stay simple:

- Project 1 status is the only actionability signal.
- `ready` means the ticket can be started now
- `in-progress` means someone is actively working it right now
- `blocked` means work cannot continue until the blocker named in the ticket is cleared

Project 1 status is the only actionability signal.

When a Project 1 item is marked `Blocked`, the project `Note` field should summarize the blocker in one short sentence so the reason is visible on the board without opening the issue.

Potential parallel candidates should stay `Ready` until they are actually picked up. We should not use `In Progress` as a placeholder for work that is merely available to start.

Open PR-backed tickets must be reflected as `In Progress` in both the issue labels and Project 1. The PM thread should treat any mismatch as a board-drift bug and fix it immediately.

Only tickets with real active execution should appear on Project 1 as `In Progress`.

For true parallel execution, use separate Codex threads with separate issues, branches, and PRs. The operating model is documented in:

- [docs/multi-thread-workflow.md](multi-thread-workflow.md)

Run the board audit when you change issue labels, Project 1 status, or PR state:

```bash
bash scripts/ci/project_board_audit.sh
```

The board audit logic lives in [scripts/ci/project_board_audit.sh](../scripts/ci/project_board_audit.sh).

---

## External Review Gate for Governing Specs

Every new governing spec must go through a time-boxed external review before being marked `approved` in `specs/governance/approved-specs.json`.

### Policy

- **Reviewers requested**: 3 external reviewers per spec
- **Review window**: 72 hours from the first review request
- **Quorum**: if fewer than 3 reviews arrive within the window, the host maintainer reviews the feedback received and makes the call — work proceeds
- **Format**: asynchronous; no meetings required
- **Template**: use [`docs/spec-reviewer-guide.md`](spec-reviewer-guide.md) to structure the review request

### Process

1. Author completes the spec draft and opens a PR.
2. Author fills in the reviewer guide template and posts it as a PR comment.
3. Author tags 3 reviewers and sets a 72-hour deadline in the comment.
4. Reviewers respond with Yes / Approve with changes / Reject using the checklist in the template.
5. After the window closes, the host maintainer resolves feedback and merges or revises.

### Spec Tickets That Require This Gate

Add a checklist item to each new spec issue:

```
- [ ] External review requested (3 reviewers, 72h window started YYYY-MM-DD HH:MM UTC)
- [ ] Review window closed / host maintainer decision recorded
```

This applies to: #329, #330, #331, #332, #335, #337, #338, #339, and any future spec tickets.

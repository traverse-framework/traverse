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

## Decision-Log Traceability Gate for Governing Specs

Governing specs are precise, testable artifacts derived from accepted product
and architecture decisions. They are not a second forum for reconsidering
decisions already made in the decision log.

### Policy

- Every new approved spec MUST cite one or more accepted decision-log entries.
- The decision log records *why* and the approved direction; the spec records
  the exact requirements, boundaries, acceptance scenarios, and verification.
- A spec MUST NOT introduce a material new product or architecture decision.
  Record that decision first, then derive the spec and tickets from it.
- The author performs a traceability check before approval: each decision is
  represented in the spec, and each spec requirement is justified by the cited
  decision or an already-approved governing spec.

### Process

1. Record or identify the accepted decision in `docs/decision-log.md`.
2. Create the issue, Project 1 item, and derived spec.
3. Link the decision, spec, implementation tickets, and downstream blockers.
4. Run the traceability and repository validation checks.
5. Mark the spec approved in `specs/governance/approved-specs.json` and merge
   the codification PR.

### Spec Ticket Checklist

```
- [ ] Accepted decision-log entry cited
- [ ] Spec requirements traced to the decision and existing governing specs
- [ ] Implementation tickets and downstream blockers linked
```

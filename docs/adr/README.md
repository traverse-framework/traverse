# Architecture Decision Records

This directory stores Architecture Decision Records (ADRs) for material technical choices in Traverse.

For the consolidated product and architecture roadmap decisions that connect multiple specs and issues, see [../decision-log.md](../decision-log.md).

## When to Write an ADR

Create an ADR for decisions that materially affect:

- capability or event contract structure
- runtime behavior
- registry behavior
- versioning rules
- merge-gating validation
- security posture
- quality standards or compatibility policy

## Minimum ADR Structure

- Title
- Status
- Date
- Context
- Decision
- Consequences
- Alternatives considered

## Status Values

- Proposed
- Accepted
- Superseded
- Rejected

## Rule

If a material architectural change is introduced without an ADR where one is required, the change should not merge.

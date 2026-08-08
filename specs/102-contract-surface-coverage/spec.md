# Feature Specification: Contract Surface Coverage

**Status**: Draft (awaits owner approval before `approved-specs.json` registration)
**Canonical governing ID**: `102-contract-surface-coverage`
**Version**: 1.0.0
**Extends**: `002-capability-contracts`, `100-capability-package-authoring`, `056-capability-publish`, `516-agent-artifact-execution`
**Input**: Issues #1014–#1016; registry#192; Decision 57; ADR-0038.
**Incident**: `core.process-comment@1.0.0` overclaimed `action` enum values and description features beyond its use-case/smoke matrix.

## Purpose

Prevent capability contracts from declaring an input/behavior surface larger than what published `use_cases` demonstrate and what package-level verification exercises. Free-text `description` and JSON Schema enums are otherwise treated as marketing, while only use cases are executable promises.

## Capability Boundary

In scope:

- Coverage rules relating `inputs.schema` discriminator enums (at minimum `action` when present) to `use_cases[].input_example`
- Publish / dry-run failure behavior when coverage is incomplete
- Authoring-guide and example-smoke conventions for enum coverage
- Cross-repo expectation that registry validation may mirror the same rule for newly ADDED contracts

Out of scope:

- NLP verification that every sentence in `description` is implemented (human honesty checklist only in v1)
- Rewriting immutable already-published registry versions
- Expanding Host ABI or guest runtime features
- Requiring 100% branch coverage of guest code beyond use-case smoke

## Requirements

- **FR-001**: When a capability contract's `inputs.schema.properties.action` (or a future listed discriminator property) declares an `enum`, every enum value MUST appear as `use_cases[i].input_example.<discriminator>` for at least one use case.
- **FR-002**: `traverse-cli capability publish` and `capability publish --dry-run` MUST fail with an actionable error listing uncovered enum values when FR-001 is violated for the contract being published.
- **FR-003**: Governed example packages that declare an `action` enum MUST include smoke fixtures covering every retained enum value (same set as FR-001).
- **FR-004**: Capability authoring documentation MUST state the coverage rule and require an explicit **Known limitations** section whenever description prose mentions behavior not represented in `use_cases`.
- **FR-005**: Schema MUST NOT list an action (or discriminator value) that the executable artifact answers only with a generic `unsupported_action` / equivalent fail-closed stub unless that failure mode itself is a documented use case with a stable `reason_code`.

## Success Criteria

- A contract that includes `resolve` in `action.enum` but has no resolve use case fails publish dry-run.
- `core.process-comment@1.0.1` (honesty bump) satisfies FR-001 for its retained enum.
- Registry can adopt an equivalent diff-based check for newly ADDED contracts without rewriting history.

## Quality Gates

- QG-001: Unit tests for the publish coverage checker (pass/fail fixtures).
- QG-002: Spec-alignment maps this spec onto CLI publish paths and authoring docs once Approved.
- QG-003: No silent weakening of host input-schema validation.

## Approval Note

This file is **Draft**. Do not treat FRs as binding in CI until the owner moves Status to Approved and registers the spec in `specs/governance/approved-specs.json`. Implementation issue #1016 remains `needs-spec` until then.

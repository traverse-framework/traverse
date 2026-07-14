# Feature Specification: Canonical Doc-Approval Pipeline

**Feature Branch**: `069-doc-approval-pipeline-canonicalization`
**Created**: 2026-07-14
**Status**: Approved
**Version**: 1.0.0
**Input**: Decision 19 in `docs/decision-log.md`, resolving Traverse #538
without duplicating the already-shipped `doc-approval.analyze@1.0.0` public
capability.

## Purpose

Define one canonical doc-approval pipeline: `doc-approval.analyze` followed by
`doc-approval.recommend`. The pipeline reuses the existing deterministic
analysis contract instead of introducing a second, incompatible extraction
capability that would duplicate public output and require a client migration.

## Relationship to Existing Specifications

| Specification | Relationship |
| --- | --- |
| 058-workflow-pipeline-execution | Supersedes only its `doc-approval.pipeline` reference table; other pipeline semantics remain unchanged. |
| 053-conditional-state-transitions | Conditional routing continues to evaluate runtime-owned analysis/recommendation output. |
| 057-embeddable-runtime-host | Embedded clients submit the canonical pipeline and render runtime-owned output. |

## Functional Requirements

- **FR-001**: `doc-approval.analyze@1.0.0` is the canonical first pipeline
  capability and retains its existing public input and output contract.
- **FR-002**: `doc-approval.recommend` MUST consume the analysis result and
  produce deterministic `recommendation`, `rationale`, and `confidence`
  fields.
- **FR-003**: The canonical `doc-approval.pipeline` invokes exactly `analyze`
  then `recommend` in order; it MUST NOT require `doc-approval.extract`.
- **FR-004**: No `doc-approval.extract` public capability or nested extraction
  schema is introduced by this pipeline.
- **FR-005**: Manifests and reference-app clients MUST render pipeline output
  owned by Traverse and MUST NOT recreate analysis or recommendation logic.
- **FR-006**: The pipeline MUST remain deterministic: the same document and
  registered artifacts yield identical output.

## Acceptance Scenarios

1. Given a document submitted to `doc-approval.pipeline`, when `analyze`
   succeeds, then `recommend` receives the analysis output and returns the
   final recommendation without a separate extraction step.
2. Given the existing `doc-approval.analyze` direct invocation, when an
   existing client uses it, then its input/output compatibility remains
   unchanged.
3. Given a bundled reference app, when it submits the pipeline, then it
   renders only runtime-owned analysis and recommendation fields.

## Out of Scope

- Adding a v2 extraction capability or a migration from `analyze@1.0.0`.
- Changing `traverse-starter.pipeline`.
- New UI behavior beyond rendering the canonical runtime result.

## Implementation Tickets

- Traverse #555 — deterministic `doc-approval.recommend` capability.
- App-References #111 — canonical pipeline integration.
- App-References #112 — canonical app/component manifests.

## Superseded Work

- Traverse #538 is superseded by the shipped `doc-approval.analyze` path and
  does not require implementation.

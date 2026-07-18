# Feature Specification: App-Facing Operational Constraints

**Feature Branch**: `021-app-facing-operational-constraints`  
**Created**: 2026-04-03  
**Status**: Superseded (2026-07-18 — see decision-log.md Decision 25; never approved, no implementation commits reference this spec ID; no specific direct successor identified)  
**Input**: The agreed decision that v0.1 needs one narrow operational-constraints slice covering first-consumer performance expectations and app-facing security/safety boundaries without attempting a full production operating model.

## Purpose

This specification defines the first app-facing operational-constraints slice for Traverse.

It narrows the broader non-functional release needs into one concrete, testable model for:

- the first app-consumable performance baseline
- the first browser- and MCP-facing security and safety boundary
- explicit v0.1 non-goals for what Traverse does not promise yet

This slice exists so the first external-consumer release can rely on a narrow but explicit operational boundary instead of vague claims like “fast enough” or “safe enough.”

This slice does **not** define a full production SLO policy, complete threat model, multi-tenant hardening, or deployment security architecture.

## User Scenarios and Testing

### User Story 1 - Evaluate Whether the First Consumer Path Is Responsive Enough (Priority: P1)

As a release steward or app developer, I want one narrow performance baseline so that the first app-consumable path can be judged against explicit expectations rather than vague impressions.

**Why this priority**: Browser-hosted app use depends on Traverse being interactively usable, but v0.1 should avoid pretending to have a full benchmark program.

**Independent Test**: A reviewer can identify the first supported responsiveness expectations from this spec alone and determine whether a validation path exists for them.

**Acceptance Scenarios**:

1. **Given** the first supported local app-consumable flow, **When** responsiveness is evaluated, **Then** the baseline covers time to first runtime update, delivery of subsequent updates, and end-to-end flow usability.
2. **Given** a performance validation path is reviewed, **When** it is inspected, **Then** it is narrow enough for v0.1 rather than a broad production SLO framework.
3. **Given** a future production deployment target exists later, **When** its performance needs differ, **Then** this slice remains a v0.1 baseline rather than a universal policy.

### User Story 2 - Preserve a Safe App-Facing Boundary for Browser and MCP Consumers (Priority: P1)

As a downstream app developer, I want the browser-facing and MCP-facing surfaces to preserve governed validation and avoid undocumented privileged behavior so that app consumption remains safe and reviewable.

**Why this priority**: The first external consumer uses Traverse as a runtime and MCP substrate, so the release must define what those public surfaces must not bypass.

**Independent Test**: A reviewer can derive the minimum required app-facing safeguards and explicit non-goals from this spec alone.

**Acceptance Scenarios**:

1. **Given** a downstream app triggers runtime execution, **When** the app-facing path is inspected, **Then** governed validation is not bypassed.
2. **Given** a downstream app uses the MCP-facing path, **When** the public surface is reviewed, **Then** it does not require undocumented privileged hooks into Traverse internals.
3. **Given** v0.1 is reviewed for security promises, **When** the slice is inspected, **Then** unsupported concerns such as full auth and multi-tenant hardening are explicitly marked out of scope.

### User Story 3 - Distinguish What v0.1 Constrains Versus What It Defers (Priority: P2)

As a project steward, I want explicit operational non-goals so that v0.1 is honest about its limits while still providing a meaningful first-consumer boundary.

**Why this priority**: Without explicit deferrals, people will infer a broader production guarantee than v0.1 actually supports.

**Independent Test**: A reviewer can tell from this spec alone which operational constraints are mandatory in v0.1 and which are intentionally deferred.

**Acceptance Scenarios**:

1. **Given** someone asks whether Traverse guarantees production-grade auth or multi-tenant hardening in v0.1, **When** this spec is inspected, **Then** the answer is explicitly “no”.
2. **Given** someone asks whether Traverse must preserve governed validation and avoid uncontrolled privileged paths in v0.1, **When** this spec is inspected, **Then** the answer is explicitly “yes”.
3. **Given** later slices need stronger performance or security models, **When** they are proposed, **Then** this spec can be extended or superseded without pretending to be that final model already.

## Scope

In scope:

- narrow performance baseline for the first app-consumable path
- narrow browser- and MCP-facing security/safety boundary
- explicit v0.1 non-goals
- validation expectations tied to those constraints

Out of scope:

- full production SLOs
- load testing strategy for all deployments
- full security architecture
- auth and identity systems
- multi-tenant isolation guarantees
- remote deployment hardening

## Functional Requirements

- **FR-001**: Traverse MUST define one narrow performance baseline for the first app-consumable path.
- **FR-002**: The performance baseline MUST include at least:
  - time to first runtime update
  - responsiveness of ordered runtime-update delivery
  - one end-to-end local app-flow usability expectation
- **FR-003**: The performance baseline MUST remain narrow enough for v0.1 and MUST NOT attempt to stand in for a full production SLO policy.
- **FR-004**: Traverse MUST define one app-facing browser and MCP safety boundary for v0.1.
- **FR-005**: The browser-facing path MUST preserve governed validation and MUST NOT introduce undocumented privileged execution bypasses.
- **FR-006**: The MCP-facing path MUST preserve governed validation and MUST NOT require downstream apps to use undocumented internal-only hooks.
- **FR-007**: This slice MUST define what app-facing browser and MCP paths must not expose in v0.1.
- **FR-008**: This slice MUST define what remains explicitly out of scope for v0.1, including at least full auth, multi-tenant hardening, and broad remote deployment security guarantees.
- **FR-009**: The operational constraints under this slice MUST remain compatible with the downstream-consumer contract and downstream validation slices rather than redefining them separately.
- **FR-010**: Approved implementation and validation under this slice MUST be checked against this governing spec before merge.

## Non-Functional Requirements

- **NFR-001 Performance Practicality**: The baseline MUST be measurable enough to guide validation, but small enough to remain practical for v0.1.
- **NFR-002 Safety Honesty**: The safety boundary MUST be explicit about what is protected now and what is not promised yet.
- **NFR-003 Determinism**: Operational validation expectations under this slice MUST remain deterministic enough for CI or repeatable review.
- **NFR-004 Maintainability**: Performance constraints and safety constraints MUST remain narrow, additive, and separable from broader future operational policies.
- **NFR-005 Portability**: The baseline and safety boundary MUST remain host-agnostic enough not to depend on one browser-specific or MCP-host-specific shortcut.

## Non-Negotiable Quality Gates

- **QG-001**: Traverse MUST NOT claim the first app-consumable release is responsive enough without one explicit v0.1 performance baseline.
- **QG-002**: Traverse MUST NOT claim the first app-consumable release is safe for downstream use if app-facing browser or MCP paths bypass governed validation.
- **QG-003**: No v0.1 release artifact may imply support for full auth, multi-tenant hardening, or hardened remote deployment security unless a later explicit governing slice adds those guarantees.
- **QG-004**: Performance and safety validation under this slice MUST remain explicit and reviewable rather than implied in prose only.

## Key Entities

- **Performance Baseline Record**: One narrow measurable statement of the first supported app-consumable responsiveness expectations.
- **Safety Boundary Rule**: One explicit browser- or MCP-facing rule describing what the public app-facing path must preserve or must not expose.
- **Operational Non-Goal Record**: One explicit statement of what v0.1 does not promise yet.
- **Operational Validation Evidence**: One record or reviewable artifact showing whether the baseline or safety rule has been checked.

## Success Criteria

- **SC-001**: The first app-consumable path has a narrow, explicit responsiveness baseline.
- **SC-002**: The first browser and MCP app-facing paths have explicit minimum safety constraints.
- **SC-003**: v0.1 makes honest operational promises without implying a full production-grade policy.

## Governing Relationship

This specification is governed by:

- `001-foundation-v0-1`
- `019-downstream-consumer-contract`
- constitution version `1.2.0`

This specification is intended to govern future implementation and validation in:

- performance-baseline definition and checks for the first app-consumable path
- browser- and MCP-facing safety-boundary enforcement
- release-readiness review for first external-consumer use

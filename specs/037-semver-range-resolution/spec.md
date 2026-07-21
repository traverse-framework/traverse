# Feature Specification: Semver Range Resolution

**Feature Branch**: `037-semver-range-resolution`
**Created**: 2026-04-19
**Status**: Approved (2026-04-19 — registered by PR #358; status header reconciled 2026-07-21 — see decision-log.md Decision 31)
**Input**: Version resolution model for Traverse capability lookup, covering semver range syntax, highest-satisfying-version selection, contract compatibility gating, exact-version pinning, and workspace-scoped resolution fallback. Unblocks GitHub issue #311.

## Purpose

This spec defines how the Traverse registry resolves a capability request that specifies a version range rather than an exact version.

The existing runtime request model (spec 006) requires capability identity and an optional version. Today the runtime treats version as an exact match. This spec introduces range-based resolution so capability consumers can express compatibility constraints (e.g., `^1.0.0`) without being pinned to a specific patch release.

The model introduced here:

- selects the highest registered version satisfying the range, within the same major version (`^` Cargo semantics by default)
- accepts Cargo/npm-style range syntax: `^1.2.0`, `>=1.0.0, <2.0.0`, `1.x`, `*`
- verifies contract compatibility before accepting a resolved version
- bypasses range logic entirely for exact-version requests
- fails explicitly on ambiguous matches (same resolved version, different digests)
- scopes resolution to the requesting workspace_id first with fallback to accessible shared workspaces

This spec does **not** define dependency graph resolution across multiple capabilities, lock file semantics, or cross-registry federation.

## User Scenarios and Testing

### User Story 1 — Range Request Resolves to Highest Satisfying Version (Priority: P1)

As a capability consumer, I want to request a capability using a semver range so that I automatically receive the highest compatible version registered in my workspace without updating my request each time a new patch is released.

**Why this priority**: Range resolution is the primary consumer-facing feature of this spec. Without it, all callers must pin exact versions, which defeats semver compatibility discipline.

**Independent Test**: Register capability `data.transform` at versions 1.0.0, 1.1.0, and 1.2.0. Submit a runtime request with range `^1.0.0`. Verify the resolver selects 1.2.0 and executes it.

**Acceptance Scenarios**:

1. **Given** capabilities registered at 1.0.0, 1.1.0, and 1.2.0, **When** a request specifies range `^1.0.0`, **Then** the resolver selects version 1.2.0 as the highest satisfying version.
2. **Given** the resolver selects 1.2.0, **When** the execution trace is produced, **Then** it records the requested range, the evaluated candidates, and the selected version.
3. **Given** a new version 1.3.0 is registered after the initial request, **When** the same range request is submitted again, **Then** the resolver now selects 1.3.0.

### User Story 2 — No Satisfying Version Fails Explicitly (Priority: P1)

As a capability consumer, I want to receive a clear error when no registered version satisfies my range so that I can diagnose version gaps without inspecting the registry manually.

**Why this priority**: Silent fallback or wrong-version execution is a correctness hazard. Explicit failure with diagnostic information is required.

**Independent Test**: Register capability `data.transform` at version 2.0.0 only. Submit a range request for `^1.0.0`. Verify the resolver returns `no_version_satisfies_range` and lists the registered versions.

**Acceptance Scenarios**:

1. **Given** only version 2.0.0 is registered, **When** a request specifies range `^1.0.0`, **Then** the resolver fails with `no_version_satisfies_range` and includes the registered versions and the requested range in the error.
2. **Given** a `no_version_satisfies_range` failure, **When** the runtime trace is produced, **Then** it records the range, the evaluated candidates, and the reason no candidate was selected.
3. **Given** a `no_version_satisfies_range` error, **When** the error is inspected, **Then** it does not expose versions from other workspaces.

### User Story 3 — Ambiguous Match Fails Explicitly (Priority: P2)

As a platform operator, I want the resolver to fail explicitly when two registrations at the same resolved version have different digests so that execution is never silently routed to an unexpected artifact.

**Why this priority**: Silent tie-breaking between different artifacts at the same version is a security and correctness risk.

**Independent Test**: Register two entries for capability `data.transform` at version 1.2.0 with different artifact digests. Submit a range request for `^1.0.0`. Verify the resolver returns `ambiguous_match` and lists both registrations.

**Acceptance Scenarios**:

1. **Given** two registrations for the same capability id and version with different digests, **When** a range resolves to that version, **Then** the resolver fails with `ambiguous_match` and lists both registrations.
2. **Given** an `ambiguous_match` failure, **When** the trace is produced, **Then** it records the matched candidates and the reason no selection was made.
3. **Given** one of the two ambiguous registrations is deregistered, **When** the range request is resubmitted, **Then** the resolver succeeds and selects the remaining registration.

### User Story 4 — Exact Version Bypasses Range Logic (Priority: P2)

As a capability consumer requiring deterministic execution, I want to pin an exact version so that range resolution logic is bypassed entirely and the named version is executed directly.

**Why this priority**: Exact pinning is necessary for reproducible builds, integration tests, and compliance scenarios.

**Independent Test**: Register capability `data.transform` at versions 1.0.0, 1.1.0, 1.2.0. Submit a request pinning exact version `1.0.0`. Verify that 1.0.0 is executed and that no range evaluation occurs.

**Acceptance Scenarios**:

1. **Given** versions 1.0.0, 1.1.0, and 1.2.0 are registered, **When** a request specifies exact version `1.0.0`, **Then** the resolver selects 1.0.0 directly without evaluating a range.
2. **Given** a request with exact version `1.0.0`, **When** version 1.0.0 is not registered, **Then** the resolver fails with `capability_not_found` (not `no_version_satisfies_range`).
3. **Given** an exact version request, **When** the execution trace is produced, **Then** it records that no range evaluation occurred and that the exact version was used.

## Edge Cases

- Malformed semver range string (e.g., `^abc`, `>=1.0.x.y`) — reject at parse time with `invalid_range_syntax` error containing the malformed input; do not attempt partial evaluation.
- Empty registry for the requested capability id — fail with `capability_not_found` before evaluating any range; do not return `no_version_satisfies_range` for a non-existent capability.
- Range `*` (any version) — valid; the resolver selects the highest registered version across all major versions in the workspace.
- Pre-release versions (e.g., `1.0.0-alpha.1`, `2.0.0-rc.3`) — excluded from range resolution unless the request explicitly opts in with a `include_prerelease: true` flag; a range of `*` does not include pre-release versions by default.
- Range request where workspace has no registrations but a shared workspace does — the resolver MUST attempt shared workspace fallback before failing; if the shared workspace satisfies the range, it MUST be used.
- Contract compatibility check fails for the highest satisfying version — the resolver MUST NOT silently fall back to a lower version; it MUST fail with `contract_incompatible` and report the failed version.
- Two registrations at the same version with identical digests — treat as the same registration (idempotent); select it without ambiguity error.
- Range `>=2.0.0, <2.0.0` (empty intersection) — reject at parse time with `invalid_range_syntax`; an unsatisfiable range is a parse error, not a resolution error.

## Functional Requirements

- **FR-001**: The runtime MUST accept a `version_range` field on capability execution requests as an alternative to an exact `version` field; the two fields are mutually exclusive.
- **FR-002**: When `version_range` is provided, the resolver MUST parse it using Cargo/npm semver range semantics supporting: `^`, `~`, `>=`, `<=`, `>`, `<`, `x` wildcards, `*`, and compound ranges with `,`.
- **FR-003**: The resolver MUST reject a malformed `version_range` at parse time with `invalid_range_syntax` before performing any registry lookup.
- **FR-004**: When a valid `version_range` is provided, the resolver MUST collect all registered versions of the requested capability id within the requesting workspace and evaluate each against the range.
- **FR-005**: The resolver MUST select the highest version among all versions satisfying the range; in the case of `^`-style ranges, only versions within the same major version MUST be considered.
- **FR-006**: Pre-release versions MUST be excluded from range evaluation unless the request includes `include_prerelease: true`.
- **FR-007**: Before accepting the resolved version as the selected candidate, the resolver MUST verify that the resolved capability's contract id and contract version are compatible with the dependency declaration in the requesting capability's contract.
- **FR-008**: If the contract compatibility check fails for the highest satisfying version, the resolver MUST fail with `contract_incompatible` and MUST NOT silently evaluate lower versions.
- **FR-009**: When no registered version satisfies the range, the resolver MUST fail with `no_version_satisfies_range` and MUST include the requested range and the list of registered versions in the error detail.
- **FR-010**: When the requested capability id has no registrations at all in the resolved scope, the resolver MUST fail with `capability_not_found` rather than `no_version_satisfies_range`.
- **FR-011**: When multiple registrations exist at the same resolved version with different artifact digests, the resolver MUST fail with `ambiguous_match` listing all conflicting registrations.
- **FR-012**: Two registrations at the same version with identical artifact digests MUST be treated as one registration; the resolver MUST select it without raising `ambiguous_match`.
- **FR-013**: When `version` (exact) is provided instead of `version_range`, the resolver MUST bypass range evaluation entirely and look up the exact registration; if not found, fail with `capability_not_found`.
- **FR-014**: Resolution MUST be scoped to the requesting `workspace_id` first (per spec 035); if no satisfying version exists in the primary workspace and the workspace has declared shared workspace access, the resolver MUST evaluate shared workspaces in declaration order.
- **FR-015**: Resolution MUST NOT cross workspace boundaries without explicit shared workspace declaration; cross-workspace resolution without authorization is a blocker defect.
- **FR-016**: The range `*` MUST be treated as matching any registered version (excluding pre-release unless opted in); the resolver MUST select the highest registered version.
- **FR-017**: The resolver MUST produce a structured resolution trace recording: requested range or exact version, candidate versions evaluated, workspace(s) searched, selected version, and failure classification if applicable.
- **FR-018**: The resolution trace MUST be included in the parent runtime execution trace (spec 006) as a nested artifact.
- **FR-019**: The resolver MUST be deterministic: the same registry state, workspace scope, and version range MUST always produce the same selected version.
- **FR-020**: All resolver error types (`invalid_range_syntax`, `capability_not_found`, `no_version_satisfies_range`, `ambiguous_match`, `contract_incompatible`) MUST be machine-readable, stable error codes suitable for programmatic handling.

## Non-Functional Requirements

- **NFR-001 Determinism**: For the same registry state and version range input, the resolver MUST always select the same version. Non-deterministic version selection is a blocker defect.
- **NFR-002 Explainability**: Every resolution outcome — success or failure — MUST produce a structured trace recording the evaluated candidates and the selection rationale.
- **NFR-003 Performance**: Range evaluation MUST complete in O(n log n) time or better with respect to the number of registered versions for a given capability id.
- **NFR-004 Testability**: Range parsing, candidate evaluation, version selection, contract compatibility checking, ambiguity detection, and workspace fallback MUST each be independently testable without a running registry.
- **NFR-005 Compatibility**: `version_range` and `version` fields MUST be mutually exclusive and versionable under semver discipline; adding range support MUST NOT break existing exact-version callers.
- **NFR-006 Correctness**: Pre-release exclusion, `^` major-version pinning, and `*` wildcard semantics MUST match Cargo semver crate behavior exactly; no custom divergence is permitted.
- **NFR-007 Maintainability**: Range parsing, candidate collection, version selection, contract compatibility gating, and workspace fallback MUST be clearly separated concerns within `traverse-registry`.

## Non-Negotiable Quality Standards

- **QG-001**: Non-deterministic version selection (same inputs, different outputs) is a blocker defect; the resolver MUST be deterministic for all range types.
- **QG-002**: Cross-workspace resolution without explicit shared workspace access is a blocker defect; workspace isolation from spec 035 MUST be respected at all resolution boundaries.
- **QG-003**: `capability_not_found` MUST be returned for non-existent capability ids; `no_version_satisfies_range` MUST NOT be returned when the capability id itself is absent.
- **QG-004**: 100% automated line coverage is required for range parsing, candidate selection, ambiguity detection, contract compatibility gating, and workspace fallback.
- **QG-005**: Semver range resolution behavior MUST align with this governing spec and fail the spec-alignment CI gate when drift occurs.

## Key Entities

- **Version Range**: A semver range expression provided on a capability execution request in place of an exact version. Parsed using Cargo/npm semver semantics.
- **Resolved Version**: The single registered version selected by the resolver as the highest version satisfying the requested range within the authorized workspace scope.
- **Candidate Version Set**: The set of all registered versions for a given capability id within the resolved workspace scope, evaluated against the requested range.
- **Exact Version Request**: A capability execution request that specifies an exact version string rather than a range. Bypasses all range evaluation; fails with `capability_not_found` if the exact version is absent.
- **Contract Compatibility Check**: The verification step that confirms the resolved capability's contract id and contract version are compatible with what the requesting capability declared as a dependency. Runs before the resolved version is accepted.
- **Ambiguous Match**: The error condition raised when two or more registrations exist at the same resolved version with different artifact digests. Requires operator intervention to resolve.
- **Resolution Trace**: The structured artifact produced by the resolver recording the range input, candidate evaluation, workspace search path, selected version, and failure classification. Embedded in the parent runtime execution trace.
- **include_prerelease**: An optional boolean flag on a version range request. When `true`, pre-release versions are included in candidate evaluation. Default: `false`.

## Success Criteria

- **SC-001**: A range request resolves to the highest satisfying version in the workspace; the resolution trace records all evaluated candidates and the selected version.
- **SC-002**: A range with no satisfying version returns `no_version_satisfies_range` with the requested range and registered version list in the error detail.
- **SC-003**: An ambiguous match (same version, different digests) returns `ambiguous_match` listing all conflicting registrations; execution is never silently routed to one of them.
- **SC-004**: An exact version request bypasses range logic entirely; the resolution trace records that no range evaluation occurred.
- **SC-005**: A malformed range string is rejected at parse time with `invalid_range_syntax` before any registry lookup.
- **SC-006**: 100% automated line coverage is achieved for all range parsing, selection, ambiguity detection, contract compatibility, and workspace fallback paths.

## Out of Scope

- Dependency graph resolution across multiple capabilities (lock file semantics)
- Cross-registry federation or external registry lookup
- Version yanking or deprecation markers
- Automatic range widening or narrowing on conflict
- Semver pre-release comparison ordering beyond Cargo crate semantics
- Range resolution for event contracts or workflow definitions (capability contracts only in this spec)
- UI or CLI tooling for range debugging

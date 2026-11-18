# Feature Specification: Production App Readiness Baseline

**Feature Branch**: `524-production-app-readiness`
**Created**: 2026-07-28
**Status**: Approved (2026-11-18) — planning baseline; each implementation slice requires its own approved successor spec.
**Version**: 1.0.0
**Approval**: Owner-approved 2026-11-18 (#1907) as the v1 planning baseline (Decision 38). Not an implementation authorization.
**Input**: Maintainer-approved brainstorming and Decisions 34–38 in `docs/decision-log.md`.

## Purpose

Define the v1 product bar for a real multi-platform application using Traverse.
An app can consume verified registry capabilities offline, execute them in an
embedded host, retain safe audit traces across restart, and upgrade local state
without data loss. This specification is a planning baseline, not an
implementation authorization or a replacement for the bounded successor specs.

## User Scenarios & Testing

### User Story 1 - Prepare and run a verified app offline (Priority: P1)

An app maintainer resolves declared capability ranges into an exact lockfile,
prepares verified artifacts into a host-owned cache, and ships an embedded app
that runs without network access.

**Independent Test**: A reviewer prepares a bundle, disconnects the host, and
executes every locked capability using only the active cache generation.

**Acceptance Scenarios**:

1. **Given** a Certified capability range and a valid registry snapshot, **When**
   the host prepares the app, **Then** it writes a lockfile and verified cache
   generation containing exact version, digest, publisher, tier, and provenance.
2. **Given** an active verified generation and no registry network access,
   **When** the app initializes and executes, **Then** it performs no network
   request and reports stale provenance when applicable.

### User Story 2 - Upgrade safely and respond to publisher lifecycle changes (Priority: P1)

An app maintainer prepares an update beside the active generation, activates it
atomically, rolls back explicitly when needed, and receives deterministic yank
or security-yank behavior.

**Independent Test**: A reviewer activates an update, rolls back to the prior
generation, and verifies normal and security-yank handling without ambiguous
runtime resolution.

**Acceptance Scenarios**:

1. **Given** a new valid lock generation, **When** the host activates it,
   **Then** the prior verified generation remains available for explicit rollback.
2. **Given** a normal yanked version, **When** preparation runs, **Then** it is
   rejected while an existing active generation remains runnable.
3. **Given** a locally known security yank past its deadline, **When** a
   Certified host executes it, **Then** execution fails closed with upgrade guidance.

### User Story 3 - Operate durable state and safe audit evidence (Priority: P1)

An authorized workspace user can retain safe traces and migrate host-owned
local state without assigning data roots, encryption keys, or tenancy to
Traverse.

**Independent Test**: A reviewer restarts a host, reads safe durable traces,
migrates a `local-datastore/1` fixture to v2 with verified backup, and confirms
that a second owner is rejected.

**Acceptance Scenarios**:

1. **Given** auditable execution, **When** it succeeds, **Then** its safe trace
   survives restart, applies the configured retention limit, and records pruning.
2. **Given** an owned v1 DataStore root, **When** the host explicitly requests
   an approved v1-to-v2 migration, **Then** source preservation, backup,
   verification, and explicit restore are available.
3. **Given** a second process opens the same root, **When** it requests access,
   **Then** it receives `store_locked` and cannot modify state.

### User Story 4 - Discover trusted capabilities across supported platforms (Priority: P2)

An app author sees Certified capabilities by default, opts into Community or
Kit content deliberately, and receives the same production guarantees across
Web, Linux/Rust, Apple, Android, and Windows/.NET.

**Independent Test**: The same locked bundle passes the certified conformance
suite on every supported platform; a non-conforming platform is marked Preview.

## Requirements

### Functional Requirements

- **FR-001**: Production bundles MUST resolve declared ranges to a committed
  lockfile with exact artifact identity and provenance before initialization.
- **FR-002**: Embedded initialization and execution MUST consume only a
  host-prepared verified cache and MUST perform no runtime network lookup.
- **FR-003**: Registry discovery MUST show Certified results by default and
  require explicit opt-in for Community and Kit/example results.
- **FR-004**: Registry records and discovery results MUST show tier, publisher,
  support state, deprecation/yank state, and provenance.
- **FR-005**: Hosts MUST prepare updates beside the active generation and
  atomically activate or explicitly roll back a verified generation.
- **FR-006**: Normal yanks MUST reject new preparation; security yanks MUST
  enforce locally known deadlines or minimum-safe-version policy for Certified hosts.
- **FR-007**: Certified tier admission MUST require verified publisher identity,
  signed immutable provenance, automated validation, conformance evidence, and
  maintainer-approved support/deprecation policy.
- **FR-008**: Durable traces MUST remain separate from DataStore, persist only
  safe evidence, be workspace-authorized, and default to 30 days or 10,000
  traces per workspace, oldest-first with pruning evidence.
- **FR-009**: The first DataStore transition MUST be from
  `local-datastore/1` to a host-owned, file-backed `local-datastore/2`, with
  verified backup and explicit restore.
- **FR-010**: DataStore and journal encryption keys, roots, tenancy mapping,
  and host-user authorization MUST remain host-owned and opaque to Traverse.
- **FR-011**: A v1 DataStore root MUST be single-writer; concurrent ownership
  MUST fail closed.
- **FR-012**: A platform may be Certified only after it passes the same
  embedded execution, verified-cache, update/rollback, and trace-restart
  conformance suite as every other Certified platform.
- **FR-013**: Normal Certified deprecation MUST provide at least 90 days'
  notice and a replacement or migration path; security lifecycle actions follow
  FR-006.

## Key Entities

- **RegistryTier**: Certified, Community, or Kit/example trust classification.
- **ResolvedLockGeneration**: Immutable app dependency resolution with exact
  identity, digest, publisher tier, and registry provenance.
- **VerifiedCacheGeneration**: Host-owned verified artifact set eligible for
  embedded initialization and explicit activation or rollback.
- **SecurityYankPolicy**: A published minimum-safe-version and/or deadline
  associated with a yanked artifact.
- **DurableTraceJournal**: Workspace-scoped safe audit evidence with retention
  and pruning events, distinct from application state.
- **DataStoreMigration**: An explicit host-directed v1-to-v2 transition with
  source verification, backup, commit, restore, and stable outcome.

## Success Criteria

- **SC-001**: A disconnected host executes 100% of a prepared locked bundle
  without runtime network access.
- **SC-002**: A reviewer can activate and explicitly roll back a verified
  generation without losing the prior runnable generation.
- **SC-003**: An auditable trace remains readable after restart and honors both
  configured age and count retention thresholds with pruning evidence.
- **SC-004**: A v1-to-v2 migration preserves all valid source records or leaves
  the prior committed representation intact on every injected failure boundary.
- **SC-005**: Every Certified platform passes one common production conformance
  suite; platforms that do not pass are visibly Preview.

## Assumptions and Boundaries

- Embedded hosting is the production path; HTTP serving remains development,
  CI, or explicitly future remote-host work.
- Community and Kit content are never implicitly trusted by production apps.
- Browser/remote synchronization, multi-process coordination, key rotation,
  and private trace-payload persistence require separate successor specs.
- This baseline is decomposed before implementation; no single ticket may claim
  the entire scope.

## Required Decomposition

1. Tiered registry trust, discovery, publisher lifecycle, and production lockfile.
2. Embedded cache preparation, activation, rollback, and security-yank enforcement.
3. Durable trace journal integration, retention, authorization, and export policy.
4. `local-datastore/2` format, explicit migration, backup/restore, encryption disclosure, and single-writer conformance.
5. Cross-platform Certified conformance suite and Preview classification.

# Feature Specification: Durable Local DataStore Integrity

**Feature Branch**: `518-durable-local-datastore`  
**Created**: 2026-07-21  
**Status**: Draft  
**Input**: Define the reachability, integrity, atomic-write, recovery, and compatibility boundary for Traverse's local `DataStore` adapter.

## Purpose

This specification defines the bounded durable-local-storage successor to the ambiguous DataStore record in Spec 032. It makes the existing local adapter a documented, embedder-owned library surface with an integrity-checked, crash-safe on-disk representation. It does not add a global runtime storage policy, cloud replication, or a new capability contract.

Spec 032's registry status and checked-in Draft header disagree. This successor is the governing surface for the local adapter's durability behavior; it does not alter capability state schemas, Lamport ordering, or merge rules.

## User Scenarios & Testing

### User Story 1 - An embedder owns durable local state (Priority: P1)

As an embedder developer, I can explicitly create a local state store at an application-chosen location, so that state ownership and retention are not silently imposed by generic capability execution.

**Independent Test**: Create a store at an application-owned temporary root, write a schema-valid state value through the public DataStore boundary, create a new store instance at that root, and read the same value.

**Acceptance Scenarios**:

1. **Given** an embedder has explicitly selected a local root, **When** it creates the local adapter and writes valid state, **Then** a new adapter at that root reads the same record.
2. **Given** ordinary runtime execution has no explicitly supplied DataStore, **When** it executes a capability, **Then** it does not create, choose, or write a local state directory.

---

### User Story 2 - Detect damaged or incomplete local state (Priority: P1)

As an operator, I receive a stable integrity failure instead of silently consuming changed or partially persisted state.

**Independent Test**: Write a record, alter its persisted payload, then read it and verify a deterministic integrity failure. Simulate an interrupted write before commit and verify the last committed record remains readable.

**Acceptance Scenarios**:

1. **Given** a committed state record has been altered, **When** it is read, **Then** the adapter returns `integrity_check_failed` and no value.
2. **Given** an interrupted write leaves a temporary record, **When** the store is reopened, **Then** the previous committed record remains the only visible value and temporary data is not listed as state.

---

### User Story 3 - Upgrade without false integrity claims (Priority: P2)

As an early adopter, I can identify legacy unhashed local state and recreate it from an authoritative source rather than having the runtime misrepresent it as verified.

**Independent Test**: Place a legacy plain state-record file in a local root and verify the adapter returns the stable integrity failure with a `legacy_unverified` reason.

**Acceptance Scenarios**:

1. **Given** a pre-governance plain state file exists, **When** it is read, **Then** the adapter rejects it as `legacy_unverified`; it does not accept or silently rewrite it.
2. **Given** an application recreates the state through the adapter, **When** it reads the new record, **Then** the new integrity-protected representation is accepted.

### Edge Cases

- A missing record remains an absent result rather than an integrity failure.
- An unknown persisted format version fails closed with `integrity_check_failed`.
- A valid digest with malformed record content fails with the existing stable serialization failure; the adapter does not return a partial record.
- Temporary files left by interrupted writes are never returned by `list_keys`.
- A failure before atomic commit leaves the earlier committed record intact.

## Requirements

### Functional Requirements

- **FR-001**: The local DataStore adapter MUST remain an explicitly constructed, embedder-owned library surface. Generic runtime execution MUST NOT select a storage root or instantiate the adapter implicitly.
- **FR-002**: Each newly committed local record MUST use the versioned `local-datastore/1` envelope containing the state record and a lowercase `sha256:` digest of its canonical serialized record content.
- **FR-003**: A read MUST verify the envelope version, record structure, and digest before returning any record. Missing, unknown, malformed, or mismatched integrity metadata MUST fail with the stable `integrity_check_failed` error and a machine-readable reason.
- **FR-004**: A write MUST create and durably flush a temporary sibling record before one atomic same-directory commit replaces the prior record. A failed write MUST NOT replace a prior committed record.
- **FR-005**: Temporary records MUST be ignored by reads and key enumeration; they MUST NOT become visible state after restart.
- **FR-006**: Plain legacy state-record files without an integrity envelope MUST fail closed with `integrity_check_failed` and reason `legacy_unverified`. Traverse MUST NOT claim their integrity or silently rewrite them.
- **FR-007**: Recreating state through the adapter is the supported migration path from legacy local files. No automatic migration is required in this slice.
- **FR-008**: The existing `DataStore` operations, state-schema validation, Lamport clock behavior, merge semantics, and capability contract shape MUST remain compatible.
- **FR-009**: The adapter documentation and integration proof MUST state that the embedding application owns root selection, retention, backup, and deletion policy.
- **FR-010**: CI MUST verify durable reopen, integrity rejection, legacy-file rejection, interrupted-write recovery, deterministic key enumeration, and no implicit runtime directory creation.

### Key Entities

- **Local DataStore Envelope**: The versioned on-disk wrapper containing one state record and its integrity digest.
- **Committed Record**: The sole state representation visible to reads and key enumeration after a successful atomic commit.
- **Legacy Unverified Record**: A former plain state file lacking required integrity metadata; it is rejected rather than trusted.
- **Embedder-owned Root**: The application-chosen location and lifecycle boundary for local durable state.

## Success Criteria

### Measurable Outcomes

- **SC-001**: 100% of newly written local records are accepted after a fresh adapter instance reopens the same root.
- **SC-002**: 100% of tampered, malformed, unknown-version, and legacy records fail without returning a state value.
- **SC-003**: In 100 simulated interrupted-write runs, the prior committed record remains readable and no temporary record appears in key enumeration.
- **SC-004**: Runtime execution without an explicitly supplied adapter creates zero local state directories in the validation environment.

## Compatibility and Migration

- The public DataStore trait and capability contract fields are unchanged.
- The persisted local-file representation changes from an undocumented plain record to `local-datastore/1`; it is an integrity boundary, not a contract version change.
- Unverified legacy files are intentionally not read. Applications recreate required values from their authoritative source, then write them through the governed adapter.
- Future automatic migration, additional local backends, and cloud or browser adapters require a successor decision.

## Assumptions

- No shipped runtime or CLI path currently constructs `LocalFileDataStore`; the adapter is a public library surface for an owning embedder.
- A same-directory atomic commit is available on the local platform supported by the adapter. Platforms without that guarantee fail the write rather than weakening the durability claim.
- SHA-256 is available in the existing runtime dependency set.

## Out of Scope

- Automatic persistence wiring into generic capability execution.
- A default root, retention policy, backup policy, or cross-application state discovery.
- SQLite, IndexedDB, cloud KV, replication, encryption-at-rest, and network synchronization.
- Automatic recovery or silent conversion of unverifiable legacy state.

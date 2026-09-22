# Feature Specification: Durable Local DataStore Integrity

**Feature Branch**: `518-durable-local-datastore`  
**Created**: 2026-07-21  
**Status**: Approved
**Version**: 1.1.0
**Input**: Define the reachability, integrity, atomic-write, recovery, and compatibility boundary for Traverse's local `DataStore` adapter.
**Amendment (2026-09-22, version 1.0.0 -> 1.1.0, approved 2026-09-22)**: Decision
100 / `#1523`. A write MUST reject a record whose canonical serialized content
exceeds a governed size ceiling, closing a gap where an unbounded write could
later produce a backup (Spec 526) that can never verify or restore. FR-013 and
the `input_limit_exceeded` failure are added; FR-012 is amended to list it.

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
3. **Given** a record's canonical serialized content exceeds the governed size ceiling (FR-013), **When** the write is attempted, **Then** it fails with `input_limit_exceeded` and no record is committed.

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

---

### User Story 4 - One owning process writes a store (Priority: P2)

As an embedder developer, I receive a stable contention failure rather than allowing two local processes to race on the same store.

**Independent Test**: Open one store at a root, attempt to open a second writer at that root from another process, and verify the second operation returns `store_locked` without changing committed state.

**Acceptance Scenarios**:

1. **Given** another process holds the root lock, **When** an adapter attempts to open it for writing, **Then** it fails with `store_locked` and a machine-readable root identifier.
2. **Given** the owning process exits or releases the adapter, **When** another process opens the root, **Then** it can acquire the lock and access the previously committed state.

### Edge Cases

- A missing record remains an absent result rather than an integrity failure.
- An unknown persisted format version fails closed with `integrity_check_failed`.
- A valid digest with malformed record content fails with the existing stable serialization failure; the adapter does not return a partial record.
- Temporary files left by interrupted writes are never returned by `list_keys`.
- A failure before atomic commit leaves the earlier committed record intact.

## Requirements

### Functional Requirements

- **FR-001**: The local DataStore adapter MUST remain an explicitly constructed, embedder-owned library surface. Generic runtime execution MUST NOT select a storage root or instantiate the adapter implicitly.
- **FR-002**: Each newly committed local record MUST use the versioned `local-datastore/1` envelope containing one generic versioned key/value state record, its explicit `classification` (`public` or `private`), and a lowercase `sha256:` digest of its canonical serialized record content. The classification is metadata only in this slice; encryption and key management are deferred.
- **FR-003**: A read MUST verify the envelope version, record structure, and digest before returning any record. Missing, unknown, malformed, or mismatched integrity metadata MUST fail with the stable `integrity_check_failed` error and a machine-readable reason.
- **FR-004**: A write MUST create and durably flush a temporary sibling record before one atomic same-directory commit replaces the prior record, then durably flush the parent directory before reporting success. A failed write MUST NOT replace a prior committed record.
- **FR-005**: Temporary records MUST be ignored by reads and key enumeration; they MUST NOT become visible state after restart.
- **FR-006**: Plain legacy state-record files without an integrity envelope MUST fail closed with `integrity_check_failed` and reason `legacy_unverified`. Traverse MUST NOT claim their integrity or silently rewrite them.
- **FR-007**: Recreating state through the adapter is the supported migration path from legacy local files. No automatic migration is required in this slice.
- **FR-008**: The existing `DataStore` operations, state-schema validation, Lamport clock behavior, merge semantics, and capability contract shape MUST remain compatible.
- **FR-009**: The adapter documentation and integration proof MUST state that the embedding application owns root selection, retention, backup, and deletion policy.
- **FR-010**: CI MUST verify durable reopen, integrity rejection, legacy-file rejection, interrupted-write recovery, deterministic key enumeration, and no implicit runtime directory creation.
- **FR-011**: The local-file adapter MUST enforce exclusive single-process ownership of an embedder root. Contention with another process MUST fail without a write as `store_locked`; multi-process coordination is out of scope.
- **FR-012**: The adapter MUST expose stable machine-readable failures for integrity (`integrity_check_failed`), schema validation (`schema_validation_error`), lock contention (`store_locked`), storage I/O (`storage_io_failed`), a failed durability commit (`durability_commit_failed`), and an oversized write (`input_limit_exceeded`, FR-013). Each failure MUST carry a non-secret machine-readable reason.
- **FR-013**: A write MUST reject a record whose canonical serialized content exceeds a governed per-record size ceiling (16 MiB) with `input_limit_exceeded`, before any temporary sibling record is created (FR-004). This keeps every committed record within the ceiling Spec 526 backup/restore also enforces (Decision 100), so a record accepted here can always later be backed up and restored.

### Key Entities

- **Local DataStore Envelope**: The versioned on-disk wrapper containing one state record and its integrity digest.
- **Committed Record**: The sole state representation visible to reads and key enumeration after a successful atomic commit.
- **Legacy Unverified Record**: A former plain state file lacking required integrity metadata; it is rejected rather than trusted.
- **Embedder-owned Root**: The application-chosen location and lifecycle boundary for local durable state.
- **Record Classification**: Explicit `public` or `private` metadata carried by every newly written envelope. It informs future policy work but does not encrypt data in this slice.

## Success Criteria

### Measurable Outcomes

- **SC-001**: 100% of newly written local records are accepted after a fresh adapter instance reopens the same root.
- **SC-002**: 100% of tampered, malformed, unknown-version, and legacy records fail without returning a state value.
- **SC-003**: In 100 simulated interrupted-write runs, the prior committed record remains readable and no temporary record appears in key enumeration.
- **SC-004**: Runtime execution without an explicitly supplied adapter creates zero local state directories in the validation environment.
- **SC-005**: In a two-process contention test, the non-owning writer returns `store_locked` and leaves the committed record unchanged in 100% of attempts.

## Compatibility and Migration

- The public DataStore trait and capability contract fields are unchanged.
- The persisted local-file representation changes from an undocumented plain record to `local-datastore/1`; it is an integrity boundary, not a contract version change.
- Unverified legacy files are intentionally not read. Applications recreate required values from their authoritative source, then write them through the governed adapter.
- Future automatic migration, additional local backends, multi-process coordination, retention, compaction, backup/restore, encryption-at-rest, key management, and cloud or browser adapters require successor decisions.

## Assumptions

- No shipped runtime or CLI path currently constructs `LocalFileDataStore`; the adapter is a public library surface for an owning embedder.
- A same-directory atomic commit is available on the local platform supported by the adapter. Platforms without that guarantee fail the write rather than weakening the durability claim.
- The supported local platform can durably flush a regular file and its containing directory. A platform that cannot provide this guarantee fails the write rather than weakening the durability claim.
- SHA-256 is available in the existing runtime dependency set.

## Out of Scope

- Automatic persistence wiring into generic capability execution.
- A default root, retention policy, backup policy, or cross-application state discovery.
- SQLite, IndexedDB, cloud KV, replication, encryption-at-rest, key management, and network synchronization.
- Automatic recovery or silent conversion of unverifiable legacy state.
- Multi-process coordination, automatic retention, compaction, backup, or restore.

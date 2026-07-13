# Feature Specification: Durable Journal Retention and Write-Path Limits

**Spec ID**: 067
**Status**: Approved
**Created**: 2026-07-12
**Input**: User-approved blocker-resolution decisions for issue #593, amending `066-durable-identity-event-delivery`.

## Context

`066-durable-identity-event-delivery` defines an append-only segmented journal
with fsync-before-acknowledgement and configurable age/size retention, but
leaves two operational questions open: how expired data is actually reclaimed,
and what happens to `publish()` when a durable write stalls. Both were
required to close issue #593's Definition of Done and are addressed here
rather than by amending the immutable 066.

## Requirements

- **FR-001**: The journal MUST roll over to a new segment once it reaches a
  configured maximum size or maximum duration, whichever occurs first; the
  default is 64 MB or 10 minutes.
- **FR-002**: Expired data MUST be reclaimed by deleting an entire segment file
  once every event within it falls outside the retention window; the journal
  MUST NOT rewrite or partially truncate a segment in place.
- **FR-003**: `publish()` MUST wait for the durable write to complete only up
  to a configured timeout, defaulting to 2 seconds; on timeout it MUST return a
  distinct `journal_write_timeout` error and MUST NOT block indefinitely.
- **FR-004**: An event that hits `journal_write_timeout` MUST be rejected, not
  delivered, and MUST NOT be silently downgraded to in-memory-only delivery;
  the caller is responsible for handling the failure.
- **FR-005**: A `journal_write_timeout` MUST produce a structured audit event,
  consistent with the existing backpressure/failure observability requirement
  in spec 036 (NFR-006).

## Out of Scope

- In-place segment compaction (rewriting a segment to remove individual
  expired records before the whole segment ages out).
- Automatic timeout tuning or adaptive backpressure based on measured disk
  latency (tracked separately as future operational-limits evaluation work,
  issues #629/#630).
- Multi-segment concurrent write coordination beyond what FR-001's rollover
  trigger requires.

## Verification

- Tests MUST prove a segment is deleted only once all its events are outside
  the retention window, and that a segment forced to roll over by size/duration
  bounds the pinned-segment overhang to one rollover period.
- Tests MUST prove `publish()` returns `journal_write_timeout` (not an
  indefinite hang) when the durable write exceeds the configured timeout, and
  that the event is not delivered through any path.
- Tests MUST prove a `journal_write_timeout` produces a structured audit event.

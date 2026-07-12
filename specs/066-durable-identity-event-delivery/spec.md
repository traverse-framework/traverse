# Feature Specification: Durable Identity-Aware Event Delivery

**Spec ID**: 066
**Status**: Draft
**Created**: 2026-07-12
**Input**: User-approved blocker-resolution decisions for issues #591 and #593.

## Context

Runtime identity must be propagated into emitted events through one canonical
runtime boundary. Replay must also survive restart without exposing raw tokens
or coupling consumers to a file layout.

## Requirements

- **FR-001**: The runtime MUST emit lifecycle events through a runtime-owned
  event-sink interface; the in-process broker MUST be one compatible sink.
- **FR-002**: Event envelopes MUST contain pinned `subject_id` and optional
  `actor_id`, and MUST NOT contain raw JWT material.
- **FR-003**: Subscriptions MUST support exact `subject_id` filtering. The
  filter MUST apply identically to live delivery and replay.
- **FR-004**: A caller MAY subscribe to its own subject only; a distinct
  audit/admin scope is required to request another subject's events.
- **FR-005**: Events MUST persist per workspace in an append-only segmented
  journal with periodic checkpoints.
- **FR-006**: A durable event MAY be acknowledged only after its journal record
  is appended and fsynced.
- **FR-007**: Cursors MUST be opaque, persisted, monotonically increasing
  sequence identifiers independent of journal segment layout.
- **FR-008**: Retention MUST be configurable by age and/or size. A request for
  compacted history MUST return `cursor_expired` and the oldest available
  cursor; it MUST NOT silently replay from another point.
- **FR-009**: Journal recovery MUST ignore only an incomplete final record from
  an interrupted write and MUST fail loudly on any malformed completed record.

## Out of Scope

- Distributed or multi-node event delivery.
- Arbitrary payload query languages and actor-based subscription filters.
- A database or pluggable storage abstraction in the first implementation.

## Verification

- Cross-boundary tests MUST prove identity reaches runtime, event envelope,
  live subscription, and replay without token leakage.
- Restart tests MUST prove cursor continuity, fsync-backed acknowledgement, and
  deterministic recovery after an incomplete final record.
- Retention tests MUST prove the stable `cursor_expired` response.

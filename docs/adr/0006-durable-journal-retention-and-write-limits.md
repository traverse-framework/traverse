# ADR-0006: Bound Durable Journal Retention and Write-Path Stalls

- Status: Accepted
- Date: 2026-07-12

## Context

ADR-0005 / spec 066 established the durable journal's append-only segments,
fsync-before-acknowledgement, and age/size retention, but left two questions
open: how expired data is physically reclaimed, and what happens to a publish
call when the durable write stalls. Both were required to close issue #593.

## Decision

Retention reclaims space by deleting whole segments once every event in a
segment is outside the retention window; segments roll over on a configured
max size or max duration (default 64 MB or 10 minutes, whichever comes first)
to bound how long a single old event can pin a segment. `publish()` waits for
the durable write up to a configured timeout (default 2 seconds); on timeout
it returns a distinct `journal_write_timeout` error and rejects the event
outright rather than silently degrading to in-memory-only delivery. The
timeout produces a structured audit event.

## Consequences

Retention behavior and write-path failure modes are now fully specified,
closing the gap left open by spec 066 and satisfying issue #593's Definition
of Done. No further spec work is required before implementation begins.

## Alternatives Considered

- In-place segment compaction instead of whole-segment deletion — rejected as
  unnecessary complexity for a first implementation that spec 066 already
  deferred.
- Unbounded blocking on a stalled durable write — rejected because it could
  hang the runtime's execution path indefinitely with no operator-visible
  signal.
- Soft-degrading to in-memory-only delivery on write timeout — rejected
  because it would silently weaken the exact guarantee this journal exists to
  provide.

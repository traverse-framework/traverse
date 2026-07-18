# ADR-0009: Retain the Durable Journal After Operational Evaluation

- Status: Accepted
- Date: 2026-07-17

## Context

Issue #630 required an evidence-led choice between retaining the initial
append-only durable journal, migrating to SQLite, or introducing a storage
provider boundary. The decision must preserve the cursor and replay semantics
defined by Specs 066 and 067.

The checked-in operational-limit harness was run successfully on 2026-07-16
for Linux host-local, Linux `fsync-pressure`, macOS host-local, and Windows
host-local profiles. Its four JSON artifacts are retained with the successful
[workflow run](https://github.com/traverse-framework/traverse/actions/runs/29504715796).

## Decision

Retain the current append-only segmented journal. Do not migrate to SQLite and
do not introduce a storage-provider boundary in the current product slice.

The comparable 1,000-event/512-byte-payload runs show host-local append p99 of
0.524 ms (Linux), 6.160 ms (macOS), and 0.711 ms (Windows); restart recovery
was 2.732-5.366 ms and replay throughput was 202,306-311,623 events/s. The
Linux `fsync-pressure` profile raised append p99 to 41.557 ms, but recovery
remained 3.278 ms and replay remained 274,122 events/s. This is above the
25 ms investigation threshold, not the two-second durable-write timeout, and
is one non-production constrained profile rather than evidence of a general
storage-engine limit.

The current journal therefore meets the measured operational need while
preserving its existing cursor and replay behavior without a migration. The
weekly/manual workflow remains the regression signal. Open a focused
remediation issue if that threshold is exceeded on two consecutive comparable
runs or reproduces on the affected storage class; reassess SQLite or a
provider boundary only with that evidence.

## Consequences

- No journal format, cursor, replay, or public API migration is required.
- No abstraction layer is added before a second backend has a demonstrated
  operational need.
- Operators retain the existing size/age retention controls and the 2-second
  fail-closed write timeout.
- The `fsync-pressure` p99 is explicitly monitored, not ignored or treated as
  a merge-blocking benchmark gate.

## Alternatives Considered

- **Migrate to SQLite now**: rejected because the measured recovery and replay
  results do not demonstrate a limitation that SQLite would solve, while a
  migration would add format and compatibility risk.
- **Introduce a provider abstraction now**: rejected because there is only one
  supported backend and no evidence-driven second backend requirement; the
  abstraction would add indirection without an exercised implementation.
- **Treat one pressure-profile threshold breach as a migration trigger**:
  rejected because the documented workflow requires a repeatable regression
  signal before changing defaults or architecture.

# ADR-0005: Emit Identity-Aware Events into a Durable Journal

- Status: Accepted
- Date: 2026-07-12

## Context

Event identity and replay are currently split from runtime execution and are
limited to in-memory retention.

## Decision

The runtime emits events through a canonical event-sink interface, carrying
pinned subject and optional actor identity. The first durable store is an
append-only segmented per-workspace journal with fsync-before-acknowledgement,
opaque persisted sequence cursors, checkpoints, and bounded retention.

## Consequences

Audit/replay survives restart with clear cursor-expiry behavior. The first
release deliberately defers a database or storage-provider abstraction, with
follow-up evaluation work required.

## Alternatives Considered

- Host-created events.
- Flexible payload/actor filters in the first release.
- SQLite or a pluggable storage layer from day one.

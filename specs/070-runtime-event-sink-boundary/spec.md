# Feature Specification: Runtime-Owned Identity-Aware Event Sink Boundary

**Feature Branch**: `070-runtime-event-sink-boundary`
**Created**: 2026-07-14
**Status**: Approved
**Version**: 1.0.0
**Input**: Decision 20 in `docs/decision-log.md`, resolving the missing
runtime-to-broker boundary for Traverse #591.

## Purpose

Define one canonical boundary for runtime lifecycle events. `Runtime` owns
construction of complete identity-aware `TraverseEvent` envelopes and emits
them through a narrow injected sink. The broker is an adapter/sink
implementation, not a dependency of the core runtime.

## Functional Requirements

- **FR-001**: Runtime MUST materialize lifecycle event envelopes at the
  runtime-owned execution boundary, before delivery or persistence adapters.
- **FR-002**: Each envelope MUST carry `subject_id` and optional `actor_id`
  from `RuntimeIdentity`, and MUST NOT carry raw token material.
- **FR-003**: Runtime MUST depend only on a narrow event-sink interface; it
  MUST NOT require an `EventBroker` concrete dependency.
- **FR-004**: Existing embedding constructors MUST retain a compatible default
  no-op or in-memory sink path.
- **FR-005**: A broker sink adapter MUST preserve the runtime envelope without
  recomputing identity or lifecycle semantics.
- **FR-006**: Live subscriptions and durable replay MUST apply identical
  `subject_id` filtering to the same canonical envelope shape.
- **FR-007**: Sink-delivery failures MUST be explicit, deterministic, and
  traceable without silently changing runtime execution results.

## Acceptance Scenarios

1. Given a runtime execution with identity, when it emits a lifecycle event,
   then the sink receives an envelope with the same subject/actor identity and
   no raw token.
2. Given an existing embedder that does not configure a broker, when it creates
   a runtime, then it retains the compatible default sink behavior.
3. Given a subject-filtered subscription, when events are received live or
   replayed from the durable journal, then both paths return the same eligible
   envelopes in the same filtering semantics.

## Out of Scope

- Distributed broker transport.
- A new query language for event payloads.
- Choosing a storage replacement for the initial durable journal.

## Implementation Tickets

- Traverse #591 — runtime identity propagation and broker adapter.
- Traverse #659 — durable replay integration after #591 lands.

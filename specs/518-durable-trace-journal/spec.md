# Feature Specification: Durable Trace Journal

**Status**: Approved

## Scope

Persist auditable execution traces through the existing append-only event journal.

## Requirements

- **FR-001**: Durable trace records MUST be canonical JSON Lines appended to the event journal and committed with `fsync`.
- **FR-002**: Auditable execution MUST fail before returning success when its trace cannot be durably written.
- **FR-003**: Recovery MUST discard only an incomplete final record and report deterministic recovery evidence.
- **FR-004**: Records MUST contain non-sensitive metadata and canonical hashes, never private trace payloads.
- **FR-005**: Retention MUST be per workspace, deterministic oldest-first, and emit `trace_pruned` evidence.

## Governing Decision

- ADR-0017

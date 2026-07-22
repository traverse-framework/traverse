# ADR-0017: Durable Trace Journal

- Status: Accepted
- Date: 2026-07-21

## Decision

Traverse will persist execution traces by reusing the append-only event journal.
Records use canonical JSON Lines and are committed with `fsync`. Trace queries
and retention rules are trace-specific layers over that journal.

Auditable executions fail before returning a result if their durable trace write
fails. Non-audited local development work may opt out through capability policy.

Recovery discards only a malformed or incomplete final record and emits a
deterministic recovery warning. It never reconstructs trace evidence.

Journal records persist non-sensitive metadata and canonical hashes only; private
trace payloads are not persisted in this slice. Retention is per workspace,
bounded by age and count/size, prunes oldest-first, and records `trace_pruned`.

## Consequences

- One durable append-only mechanism serves events and traces.
- Trace storage remains portable and inspectable without a database dependency.
- Storage failures are visible integrity failures for audited execution.
- Encrypted private-payload persistence is a future, separate capability.

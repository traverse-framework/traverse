# ADR-0066: Server-Owned Application Availability

- Status: Accepted
- Date: 2026-09-09
- Governing spec: `133-app-availability-lifecycle` (Proposed)
- Related issue: #1324

## Context

Runtime execution state and a manifest's business workflow do not answer
whether `serve` has materialized a registered app. Reopening an unresolved
source manifest caused Registry-backed apps to disappear silently despite
successful registration.

## Decision

Adopt a third, server-owned model with exactly `loading`, `ready`, and
`failed`. Its only v1 transitions are `loading → ready` and `loading → failed`.
Retain transition evidence only for the active process; expose it through
structured diagnostics and a read-only app-status endpoint. Read persisted,
resolved registration state rather than source manifests. WASM remains
demand-loaded and per-invocation verified.

## Consequences

The model makes app materialization observable without coupling business
workflows to server health or making startup compile every declared capability.
It adds no reload, degradation, durable lifecycle journal, or Registry fetch
path. A failed registered app is `503 app_unavailable`, never silently 404.

## Alternatives Considered

- Fold availability into runtime lifecycle — rejected: its ownership and scope
  differ from one execution attempt.
- Fold it into app workflow — rejected: an app cannot authoritatively report
  a state machine that failed before it was loaded.
- Fail all `serve` startup for one app — rejected: unrelated valid apps remain
  usable.
- Re-resolve source manifests at startup — rejected: duplicates admission,
  risks mutable drift, and scales poorly for large apps.

## Approval Evidence

Enrico approved this ADR and Spec `133-app-availability-lifecycle` on
2026-09-09 in the governing brainstorm for Traverse #1324.

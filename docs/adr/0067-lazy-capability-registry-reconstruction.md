# ADR-0067: Lazy Capability-Registry Reconstruction

- Status: Accepted
- Date: 2026-09-09
- Governing spec: `134-lazy-capability-registry-reconstruction` (Proposed)
- Related issue: #1328

## Context

Workspace startup currently reconstructs all component contract registrations.
Large applications pay this cost even when most capabilities are unused.

## Decision

Use an immutable persisted metadata index at startup and hydrate verified
contracts on demand. Cache successful hydration in a bounded process-local LRU
and coalesce concurrent first use per immutable key. Keep failures
command/workflow-scoped, workflows just-in-time by reached step, and updates
restart-only.

## Consequences

Startup cost avoids full contract reconstruction while retaining all routing,
policy, placement, authorization, and workflow-reference facts eagerly. No
Registry network operation, automatic reload, or persisted cache is introduced.

## Alternatives Considered

- Eagerly reconstruct every contract — rejected for large-app startup cost.
- Author-selected eager/lazy modes — rejected as premature readiness policy.
- Automatic file-watch reload — rejected for snapshot/cache races.

## Approval Evidence

Enrico approved this ADR and Spec 134 on 2026-09-09 during the #1328
governing brainstorm.

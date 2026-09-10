# Feature Specification: Lazy Capability-Registry Reconstruction

**Status**: Approved
**Canonical governing ID**: `134-lazy-capability-registry-reconstruction`
**Extends**: `046-public-cli-app-registration`, `133-app-availability-lifecycle`
**Decision evidence**: Traverse #1328 decisions D1–D7

## Purpose

Make large registered applications responsive at `serve` startup without
weakening trusted admission, deterministic discovery, workflow validation, or
per-invocation artifact verification.

## Model

Registration remains the sole Registry-resolution admission point. At startup,
`serve` loads one immutable workspace snapshot and an eager trusted metadata
index. The index contains component/app identity and versions, capability id,
contract/artifact digests and references, workflow references, and every
immutable routing, placement, policy, and authorization fact needed before
contract hydration. It excludes contract bodies/schemas and WASM bytes.

## Requirements

- **FR-001**: Startup MUST build the index solely from persisted resolved
  workspace registration state. It MUST NOT fetch, sync, resolve mutable
  Registry references, or mutate workspace state.
- **FR-002**: Discovery, placement, authorization, policy, and workflow
  topology decisions before hydration MUST use only immutable indexed facts.
- **FR-003**: A full contract MUST be read, digest-verified, parsed, and
  registered only when a command or reached workflow step requires it.
- **FR-004**: Hydrated contracts MUST be stored in a host-configured bounded,
  process-local LRU keyed by immutable capability identity and contract digest.
  Only successful verified hydrations enter the cache; eviction is safe and
  deterministic. The cache is not persisted or app-configurable.
- **FR-005**: Concurrent hydration for one key MUST be single-flight. Waiters
  receive the leader outcome. Failures MUST NOT be negatively cached.
- **FR-006**: Missing, corrupt, or digest-mismatched contract/artifact state
  discovered while hydrating MUST fail only the dependent command/workflow with
  a stable secret-free diagnostic. It MUST NOT alter app availability.
- **FR-007**: Workflows MUST validate topology and capability identities from
  the index at startup, and hydrate only reached steps. Hydration failure ends
  that execution with ordered trace evidence.
- **FR-008**: A running server uses an immutable snapshot. Registration changes
  take effect only on restart or a future governed reload operation.
- **FR-009**: Evidence MUST distinguish index lookup, cache hit, hydration
  leader, coalesced waiter, eviction, and hydration failure; it MUST not expose
  private paths, secrets, Registry URLs, contract bodies, or artifact bytes.

## Acceptance Scenarios

1. A 100-component app starts by indexing persisted metadata without parsing
   all contracts or loading WASM; its first selected capability hydrates once.
2. Concurrent first use of one capability coalesces to one verified hydration.
3. A corrupt contract fails only its command; unrelated indexed capabilities
   remain usable.
4. A branching workflow does not hydrate unreached branches.
5. Re-registration during a running process has no effect until restart.

## Quality Gates and Non-goals

Conformance MUST cover cache bounds/eviction, concurrency, trace ordering,
workflow behavior, redaction, restart snapshots, and supported host OSes.
This spec excludes developer-selected load modes, dynamic dependency injection,
negative caching, hot reload, durable cache/history, network Registry access,
and degraded/optional app lifecycle semantics.

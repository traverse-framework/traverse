# ADR-0016: Add a Companion Public Embedded Trace Surface

- Status: Accepted
- Date: 2026-07-21
- Governing spec: Draft `517-embedded-trace-api`
- Owner: Traverse maintainers
- Review by: 2026-10-21

## Context

Trace Explorer needs to inspect the current local embedded runtime session
without `traverse-cli serve`. Traverse already has tiered trace evidence and
MCP trace queries, but no portable public embedder trace surface. The current
`BundleEmbedder` uses `Runtime` directly, rather than the older
`PlacementRouter` and its `TraceStore`; it therefore cannot safely publish the
existing store as the embedded source of truth. `RuntimeTrace` is also unsafe
as a public DTO because it carries the submitted request and result.

The existing `embedder-api/1.0.0` is public and implemented as a Rust trait.
Adding required methods to that trait would break external implementations.
The baseline IDL also intentionally limits its operation set. A direct trace
store handle would be Rust-specific and would expose an internal storage
boundary rather than a portable application contract.

## Decision

Introduce a separately versioned companion contract,
`embedded-trace-api/1.0.0`. It is an optional additive capability of an
embedder package, not a modification of `embedder-api/1.0.0`. Packages
advertise its version alongside their existing release evidence; applications
that need trace browsing require a compatible companion version, while all
baseline consumers retain their existing behavior.

The companion contract has two operations:

1. `trace.list`: return a bounded, cursor-paged sequence of public trace
   summaries for the current embedder instance.
2. `trace.get`: return a public trace detail by identifier.

Results are newest completion first, with trace identifier ascending as the
tie-breaker. A list is scoped to one initialized embedder, uses opaque cursors,
and reports stable `invalid_cursor`, `trace_not_found`,
`trace_api_unavailable`, and incompatible-version outcomes as applicable.

The host derives and retains a safe public projection when an embedded
capability or workflow submission completes. It retains a bounded,
process-local history with documented deterministic eviction and clears it at
shutdown or reinitialization. The initial contract does not promise durable or
cross-restart history and does not introduce live trace streaming.

The public projection may include identifiers, target identity, completion
time, status, safe duration, selected target/version, placement, safe phase
codes, violation classifications, and stable error codes. It must never
include raw request input, raw output, caller identity, correlation metadata,
private trace entries or hashes, raw telemetry attributes, or unfiltered error
messages/details. Privileged/private diagnostics require a separate decision.

In Rust, this is represented by a separate extension trait or capability
object rather than new required methods on `TraverseEmbedderApi`. Equivalent
language bindings expose the same companion operations. The production package
and deterministic test double implement the same safe data and failure
semantics; test fixtures supply diagnostic records rather than UI-generated
business logic.

## Consequences

- Trace Explorer gains a portable no-sidecar path without forcing every
  existing embedder consumer to upgrade or breaking external Rust implementers.
- A host can ship the baseline API without the companion API; trace-aware
  consumers receive a deterministic compatibility failure instead of a hidden
  HTTP fallback.
- Public diagnostic safety becomes an explicit projection boundary rather than
  an accidental serialization of `RuntimeTrace` or `TraceStore` internals.
- The first implementation must provide a Web-host conformance proof and a
  Trace Explorer-equivalent local browse proof. Other host packages may add
  compatible support later.
- Durable trace persistence remains independently decidable and cannot be
  implied by the public API's local-session semantics.

## Alternatives Considered

- Extend `embedder-api/1.0.0` directly: rejected because the baseline operation
  set is versioned and required Rust trait methods would be breaking for
  external implementers.
- Expose `TraceStore` or `RuntimeTrace` directly: rejected because neither is a
  portable consumer contract and `RuntimeTrace` can contain unsafe payload data.
- Reuse the HTTP trace endpoint in embedded clients: rejected because it keeps
  the sidecar exception and contradicts the embedded production path.
- Make trace durability a prerequisite: rejected because local diagnostic
  browsing is valuable before cross-restart retention is decided.
- Add live streaming now: deferred because list and detail satisfy the current
  consumer need with a smaller compatibility and privacy surface.

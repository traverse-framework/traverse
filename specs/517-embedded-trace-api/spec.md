# Feature Specification: Embedded Trace API

**Feature Branch**: `517-embedded-trace-api`
**Created**: 2026-07-21
**Status**: Approved (2026-07-21 — see decision-log.md Decision 32 and ADR-0016)
**Input**: Public, portable trace browsing for a local embedded Traverse session
without an HTTP sidecar, raw payload exposure, or a durable-history requirement.

## Purpose

This specification defines an additive public trace-query surface for an
application that owns an initialized Traverse embedder. It lets a diagnostic
consumer such as Trace Explorer list and open runtime-owned execution traces
from that local session without starting `traverse-cli serve`.

The surface is a companion to, rather than a revision of,
`embedder-api/1.0.0`. Existing embedder operations and consumers remain
compatible. The feature is intentionally public-tier only and process-local:
it does not expose raw request or output data, private trace hashes, HTTP
transport, live streaming, or cross-restart history.

## User Scenarios & Testing

### User Story 1 - Browse a local embedded session (Priority: P1)

As a Trace Explorer user, I want to list and open traces produced by the
current embedded session so that I can understand what the runtime executed
without attaching to an HTTP sidecar.

**Why this priority**: This is the App-References unblocker. A list and detail
view are the smallest independently useful diagnostic capability.

**Independent Test**: An embedded session executes successful and failed
targets. A consumer lists the resulting summaries, opens one by ID, and sees
runtime-owned public evidence with no sidecar running.

**Acceptance Scenarios**:

1. **Given** an initialized embedded session has completed executions, **When**
   a consumer lists traces, **Then** it receives a deterministically ordered,
   paged set of public summaries for that session only.
2. **Given** a returned trace identifier, **When** a consumer opens it,
   **Then** it receives the corresponding safe detail record.
3. **Given** the embedder has been shut down and initialized again, **When** a
   consumer lists traces, **Then** it does not receive history from the former
   process-local session.

---

### User Story 2 - Inspect diagnostics without exposing payloads (Priority: P1)

As an application operator, I want the public trace view to explain an
execution outcome without disclosing submitted inputs, produced outputs, user
identity, private hashes, or unfiltered error details.

**Why this priority**: A diagnostic surface must not turn the embedded host
into a data-exfiltration path.

**Independent Test**: Execute a target using unique secret-like input and
output values. Inspect every returned summary and detail field and verify none
of those values, their private hashes, or caller identity are present.

**Acceptance Scenarios**:

1. **Given** an execution has sensitive input or output, **When** its public
   detail is retrieved, **Then** no raw input, output, hash, caller identity,
   correlation value, or unfiltered error detail is returned.
2. **Given** an execution fails, **When** its detail is retrieved, **Then** the
   consumer receives a stable failure classification and safe phase evidence.
3. **Given** a private trace record exists internally, **When** a public
   embedded consumer requests the trace, **Then** no private-tier field is
   returned.

---

### User Story 3 - Adopt the extension without breaking current hosts (Priority: P2)

As an application maintainer, I want to detect whether an embedder package
supports trace browsing so that applications already using the baseline
embedder API continue to work unchanged.

**Why this priority**: The baseline embedder API is public and versioned;
diagnostic functionality must be additive.

**Independent Test**: A baseline-only consumer performs all existing embedder
operations unchanged. A trace-aware consumer detects the companion version and
uses its deterministic test double to exercise list and detail behavior.

**Acceptance Scenarios**:

1. **Given** a baseline `embedder-api/1.0.0` consumer, **When** a package adds
   this feature, **Then** its existing initialization, submission, lifecycle,
   and subscription behavior remains compatible.
2. **Given** a consumer that requires the companion trace API, **When** the
   host does not advertise a compatible version, **Then** it receives a stable
   compatibility failure rather than falling back to HTTP.
3. **Given** a consumer test, **When** it uses the deterministic trace test
   double, **Then** it can validate rendering and error handling without
   generating business fields outside Traverse.

### Edge Cases

- A trace is requested after bounded local retention has evicted it: return a
  stable not-found result; do not return a different trace.
- A supplied page cursor is malformed, stale, or from another session: reject
  it with a stable invalid-cursor result.
- Two traces have the same completion timestamp: their ordering remains stable
  through a documented identifier tie-breaker.
- A target fails before producing a normal result: a safe trace summary and
  detail still identify the failure classification when runtime evidence exists.
- A consumer requests detail from a stopped host: return the existing stable
  stopped/unavailable boundary failure; do not restart the runtime.

## Requirements

### Functional Requirements

- **FR-001**: Traverse MUST publish a versioned, portable companion contract
  named `embedded-trace-api/1.0.0`; it MUST be additive to
  `embedder-api/1.0.0` and MUST NOT change any baseline operation.
- **FR-002**: A compatible host MUST let its owning application list public
  trace summaries for completed executions in the current embedded session.
- **FR-003**: A compatible host MUST let its owning application retrieve one
  public trace detail record by trace identifier.
- **FR-004**: List results MUST support a bounded page size and opaque cursor;
  ordering MUST be deterministic as newest completion first, then trace
  identifier ascending for ties.
- **FR-005**: Every returned summary MUST include a trace identifier, execution
  identifier, target identity, completion time, outcome, and safe duration or
  equivalent completion evidence.
- **FR-006**: Every returned detail MUST contain only a documented safe
  projection of runtime-owned evidence, including safe execution phases,
  selected target information, placement information where available, and
  stable failure or violation classifications where applicable.
- **FR-007**: The public companion API MUST NOT return raw inputs, raw outputs,
  caller identity, correlation metadata, private trace hashes, private trace
  entries, unfiltered error messages or details, or raw telemetry attributes.
- **FR-008**: The host MUST retain only a bounded process-local trace history.
  Retention and eviction behavior MUST be documented, visible to consumers,
  deterministic, and reset on shutdown or reinitialization.
- **FR-009**: The host MUST record a safe public projection for each completed
  capability or workflow submission that has runtime execution evidence; the
  consumer MUST NOT synthesize trace fields from UI state.
- **FR-010**: The companion API MUST provide stable machine-readable failures
  for trace not found, invalid cursor, unavailable/stopped host, and
  incompatible trace API version.
- **FR-011**: Each public embedder package that advertises the companion API
  MUST ship a deterministic trace test double or fixture harness implementing
  the same list, detail, ordering, eviction, and error semantics.
- **FR-012**: Trace Explorer and other consumers MUST use the companion API for
  their embedded production path and MUST NOT fall back to `traverse-cli serve`
  or HTTP trace retrieval.

### Key Entities

- **Embedded Trace Session**: The bounded, process-local trace history owned by
  one initialized embedder instance.
- **Trace Summary**: The safe, list-oriented public record for one completed
  execution.
- **Trace Detail**: The safe, runtime-owned diagnostic projection returned for
  a single trace identifier.
- **Trace Cursor**: An opaque continuation token for one deterministic trace
  listing session.
- **Trace API Capability**: The advertised companion contract version that a
  consumer uses to determine whether trace browsing is supported.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A Trace Explorer-equivalent consumer can list and open 100 local
  traces from an embedded session without any HTTP sidecar process.
- **SC-002**: Automated tests demonstrate that 100% of public trace responses
  omit supplied unique input/output values, caller identity, private hashes,
  and unfiltered error details.
- **SC-003**: Repeating the same list request against unchanged retained traces
  yields the same identifiers in the same order in 100 consecutive runs.
- **SC-004**: Baseline embedder conformance remains green for every current
  baseline operation, and the trace test double passes the companion conformance
  scenarios.
- **SC-005**: A package that lacks a compatible companion version fails with a
  stable compatibility result and makes zero HTTP requests.

## Assumptions

- The owning embedded application is the authorization boundary for the
  public-tier trace API; privileged/private trace access requires a separate
  approved decision.
- The first supported consumer is the Web embedder used by Trace Explorer;
  other host packages may add the companion API in later compatible releases.
- Process-local history is sufficient for the initial diagnostic use case.
  Durable cross-restart trace history remains governed by the separate durable
  trace-store decision package.
- Live trace streaming is out of scope for this version. Consumers may refresh
  the deterministic list after existing runtime events or user actions.
- This specification supersedes no approved specification. It complements
  `057-embeddable-runtime-host`, `068-public-platform-embedder-packages`, and
  the approved registry record for `012-execution-trace-tiered`.

## Out of Scope

- HTTP endpoints, sidecar discovery, or a sidecar fallback.
- Private-tier trace retrieval, raw payload inspection, or privileged
  diagnostic authorization.
- Durable trace storage, cross-restart browsing, export, or retention beyond
  the current embedded process.
- Live trace streaming, remote trace aggregation, and cross-application trace
  search.
- UI business logic, consumer-specific computed fields, or App-References
  manifest changes.

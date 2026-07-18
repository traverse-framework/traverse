# Feature Specification: Local Browser Adapter Transport

**Feature Branch**: `019-local-browser-adapter-transport`  
**Created**: 2026-04-02  
**Status**: Superseded (2026-07-18 — see decision-log.md Decision 25; never approved, no implementation commits reference this spec ID; superseded by `019-downstream-consumer-contract`)  
**Input**: Issue `#119`, the approved browser runtime subscription contract, the first app-consumable gap analysis, and the need for one concrete local browser transport that does not redefine core runtime semantics.

## Purpose

This spec defines the first concrete local browser adapter transport for Traverse.

It binds the already-approved `013-browser-runtime-subscription` message contract to one local browser-consumable transport so that:

- one browser app can request one runtime subscription locally
- the app can receive the governed ordered lifecycle, state, trace, and terminal messages over a real browser-friendly stream
- transport setup failures stay deterministic without reopening the core runtime contract

This slice governs the local adapter transport only. It does **not** redefine browser subscription payloads, runtime semantics, replay, multiplexing, auth, or distributed delivery.

## User Scenarios and Testing

### User Story 1 - Create One Local Browser Subscription (Priority: P1)

As a browser app, I want to create one local subscription for one runtime outcome so that I can render real Traverse progress and results instead of a fixture-only session.

**Why this priority**: This is the missing governed step between the transport-agnostic subscription contract and a real app-consumable browser flow.

**Independent Test**: Submit one valid local adapter create request for a known runtime outcome, then open the returned stream URL and verify the adapter emits the governed browser subscription messages in order.

**Acceptance Scenarios**:

1. **Given** a valid subscription create request with `request_id`, **When** the local browser adapter accepts it, **Then** it returns one local stream URL for that targeted outcome.
2. **Given** a valid subscription create request with `execution_id`, **When** the local browser adapter accepts it, **Then** it returns one local stream URL for that targeted outcome.
3. **Given** a browser app opens the returned stream URL, **When** the runtime outcome is materialized, **Then** the adapter emits the already-governed browser subscription messages without renaming them.

### User Story 2 - Fail Deterministically During Local Setup (Priority: P1)

As a browser integrator, I want local adapter setup failures to be deterministic and machine-readable so that app code can handle invalid setup cleanly.

**Why this priority**: A live browser app should not guess whether setup failed because of invalid input, a missing runtime outcome, or adapter misuse.

**Independent Test**: Submit invalid create requests and valid-but-missing selectors, then verify the local adapter returns one deterministic setup error response and does not expose a partial stream.

**Acceptance Scenarios**:

1. **Given** a create request with invalid browser-subscription payload, **When** the adapter validates it, **Then** it returns one governed setup error response with `invalid_request`.
2. **Given** a syntactically valid create request that targets a missing runtime outcome, **When** the adapter evaluates it locally, **Then** it returns one governed setup error response with `not_found`.
3. **Given** a browser app requests a stream URL that was never created, **When** the adapter receives it, **Then** it returns one deterministic adapter-level `not_found` response.

### User Story 3 - Keep Core Semantics Separate From Adapter Concerns (Priority: P2)

As a reviewer, I want the transport slice to keep adapter responsibilities separate from core runtime semantics so that browser delivery can evolve without changing the governed runtime model.

**Why this priority**: Traverse is explicitly avoiding a mandatory sidecar or browser-specific runtime core.

**Independent Test**: Inspect the adapter spec and confirm that the create/request lifecycle, stream transport, and local URL shape are adapter concerns while payload ordering and meanings still come from `013-browser-runtime-subscription`.

**Acceptance Scenarios**:

1. **Given** the local adapter transport spec, **When** it refers to streamed runtime payloads, **Then** it reuses the approved browser subscription message kinds and ordering instead of defining alternate payloads.
2. **Given** the local adapter transport spec, **When** it describes local URLs and stream setup, **Then** those concerns stay explicitly adapter-scoped.
3. **Given** future browser transports are added, **When** they reuse the same subscription payloads, **Then** this slice does not force changes to the governed runtime message contract.

## Edge Cases

- What happens when both `request_id` and `execution_id` are provided?
- What happens when neither selector is provided?
- What happens when the local stream URL is requested after the stream has already completed?
- What happens when the targeted runtime result is terminal-error rather than successful completion?
- What happens when the browser client disconnects before terminal completion?
- What happens when the local adapter host is reachable but the targeted runtime outcome does not exist?

## Functional Requirements

- **FR-001**: The local browser adapter MUST expose one create-subscription operation over local HTTP.
- **FR-002**: The create-subscription request payload MUST embed the approved `013-browser-runtime-subscription` request artifact unchanged.
- **FR-003**: The create-subscription operation MUST accept exactly one selector through the embedded governed browser subscription request.
- **FR-004**: A successful create-subscription response MUST return one deterministic local stream URL for one targeted runtime outcome.
- **FR-005**: The stream URL MUST be consumable through Server-Sent Events (`text/event-stream`) over local HTTP GET.
- **FR-006**: The Server-Sent Events payload MUST carry the already-governed browser subscription message artifacts from `013-browser-runtime-subscription` without renaming or reshaping them.
- **FR-007**: The first successful streamed payload MUST still be `subscription_established` and the last successful streamed payload MUST still be `stream_completed`, as governed by `013-browser-runtime-subscription`.
- **FR-008**: Invalid create requests MUST fail before stream creation with one machine-readable setup error response and MUST NOT expose a partial stream URL.
- **FR-009**: Valid create requests that target a missing runtime outcome MUST fail before stream creation with one machine-readable setup error response using `not_found`.
- **FR-010**: The adapter MUST expose one deterministic adapter-level `not_found` response when a client requests a nonexistent stream URL.
- **FR-011**: The local adapter MUST target one runtime outcome per created stream and MUST NOT multiplex multiple outcomes onto one stream in this slice.
- **FR-012**: The local adapter MUST remain local-only in this slice; remote exposure, auth, replay, and distributed fan-out are out of scope.
- **FR-013**: The adapter transport MUST remain compatible with runtime outcomes produced by both executable and workflow-backed capabilities.
- **FR-014**: The adapter MUST document what belongs to the adapter layer versus the core governed browser subscription contract.

## Non-Functional Requirements

- **NFR-001 Determinism**: The same valid create request and same targeted runtime outcome MUST produce the same ordered stream payload sequence.
- **NFR-002 Browser Compatibility**: The first slice MUST use a browser-native streaming mechanism that can be consumed without undocumented custom browser extensions.
- **NFR-003 Separation of Concerns**: Local URL shapes, HTTP status handling, and stream framing MUST be adapter concerns; runtime message semantics MUST remain governed by `013-browser-runtime-subscription`.
- **NFR-004 Testability**: Adapter request validation, setup errors, and stream framing MUST be separable enough to drive deterministic smoke or test coverage in implementation.
- **NFR-005 Extensibility**: Future transports MAY supersede or add to this local adapter transport, but they MUST NOT break the approved browser subscription payload contract without explicit new governance.

## Non-Negotiable Quality Standards

- **QG-001**: No local adapter implementation may invent alternate runtime message kinds for browser subscription payloads.
- **QG-002**: No valid local stream may omit the governed trace artifact or terminal result payload.
- **QG-003**: Invalid or missing targets MUST fail with deterministic setup errors rather than partial stream creation.
- **QG-004**: The adapter slice MUST stay local-only and MUST NOT silently introduce remote exposure or auth behavior.
- **QG-005**: The spec MUST leave core runtime semantics transport-agnostic and browser-adapter concerns clearly separated.

## Key Entities

- **Local Browser Subscription Create Request**: One HTTP request whose body contains the approved browser runtime subscription request artifact.
- **Local Browser Subscription Create Response**: One successful setup response returning the local stream URL and targeted identifiers.
- **Local Browser Subscription Setup Error**: One machine-readable create/setup failure response for invalid input or missing runtime outcomes.
- **Local Browser Subscription Stream**: One SSE stream that carries the governed browser runtime subscription messages.
- **Local Browser Adapter Not Found Error**: One adapter-level response for a requested stream URL that does not exist.

## Success Criteria

- **SC-001**: One browser app can create one local browser subscription and receive the governed ordered runtime stream over SSE.
- **SC-002**: Invalid or missing-target setup fails before stream creation with deterministic machine-readable errors.
- **SC-003**: The adapter slice enables the next implementation step of upgrading the React browser demo from fixture-driven to live-consumable.
- **SC-004**: The local transport slice stays narrow enough that it does not reopen browser subscription payload governance.

## Out of Scope

- Auth or authorization
- Replay or resume
- Subscription multiplexing
- Distributed fan-out
- Remote browser exposure
- WebSocket transport
- Browser UI presentation components

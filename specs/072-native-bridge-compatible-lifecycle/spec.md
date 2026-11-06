# Feature Specification: Native Bridge Compatible-Capability Lifecycle

**Feature Branch**: `072-native-bridge-compatible-lifecycle`  
**Created**: 2026-07-16  
**Status**: Approved  
**Version**: 1.1.0  
**Input**: Decision 24, ADR-0008, and Traverse #716.

**Amendment (2026-11-06, version 1.0.0 -> 1.1.0, Decision 127, issue #1819)**:
From `#1422` to `#1819`, `runtime.wasm` exported the bridge 1.1 lifecycle as
stubs that returned `-1`. Decision 127 keeps the lifecycle runtime-owned on
every host and adds FR-010 to FR-014. They cover the compatible init header,
the platform allowlist, canonical start input, one running instance, and the
error vocabulary. The bridge stays `runtime-wasm-bridge/1.1.0` (ABI `10_100`):
every addition is an optional JSON member, and native hosts pin the ABI
exactly.

## Purpose

Complete the runtime-owned native bridge beneath `embedder-api/1.0.0` by
governing compatible-capability start, stop, and kill. This specification is
additive to Spec 071 and publishes `runtime-wasm-bridge/1.1.0`.

## Functional Requirements

- **FR-001**: Bridge 1.1 MUST retain every bridge 1.0 export and add
  `traverse_compatible_start`, `traverse_compatible_stop`, and
  `traverse_compatible_kill` with the signatures in the ABI manifest.
- **FR-002**: Start input MUST be JSON `{ "capability_id", "input" }`. Success
  MUST return `{ "instance_id", "status": "started", "error": null }`.
- **FR-003**: Stop and kill input MUST be JSON `{ "capability_id",
  "instance_id" }`. A null instance identifier selects the active instance for
  that capability. Success returns the resolved identifier and respectively
  `stopped` or `killed`.
- **FR-004**: The runtime MUST generate stable opaque instance identifiers and
  own the active-instance state. Native hosts MUST NOT synthesize identifiers
  or lifecycle transitions.
- **FR-005**: Start, stop, and kill MUST emit runtime-owned ordered events using
  the same queue and descriptor rules as `traverse_next_event`.
- **FR-006**: An empty capability identifier or malformed JSON MUST return
  `invalid_input`. An inactive or mismatched instance MUST return
  `invalid_state` with structured code `compatible_instance_not_active`.
- **FR-007**: Kill and stop cleanup MUST release the active instance exactly
  once. A repeated operation returns the same deterministic inactive-instance
  error without additional events.
- **FR-008**: Shutdown MUST kill all remaining compatible instances before
  returning `stopped`; its result MUST include the ordered list of killed
  instance identifiers for traceability, and no instance may remain active.
- **FR-009**: A package certifying complete `embedder-api/1.0.0` conformance
  MUST require bridge version `>=1.1.0,<2.0.0` and record the exact version in
  release evidence.
- **FR-010**: A compatible capability's `traverse_init` header MUST declare
  `execution_mode: "compatible"`, a non-empty `platforms` list, and the host's
  `host_platform`, and MUST NOT carry artifact bytes. An `execution_mode`
  other than `wasm`, `compatible`, or absent fails init.
- **FR-011**: `traverse_compatible_start` MUST fail with `invalid_state`
  (`-1`) and error `platform_not_supported` when `host_platform` is not in
  `platforms`, and emit no event.
- **FR-012**: `traverse_compatible_start` MUST apply Spec 1722 to the request.
  It rejects what JCS can't represent with `invalid_input` (`-2`) and an
  `input_not_canonicalizable` error, and on success returns
  `canonical_input`, the RFC 8785 text of `input`. The host MUST hand those
  bytes to the platform wrapper unchanged.
- **FR-013**: At most one instance of the capability runs at a time. A start
  while one runs fails with `invalid_state` and error
  `compatible_instance_active`. Instance identifiers are never reused within
  one initialization.
- **FR-014**: A start, stop, or kill whose `capability_id` isn't the
  initialized compatible capability fails with `invalid_input` and error
  `capability_not_compatible`. `traverse_submit` on a compatible capability is
  rejected with `compatible_lifecycle_required`. Lifecycle events carry
  `type`, `capability_id`, and `instance_id`. Shutdown of a runtime with no
  compatible capability keeps the bridge 1.0 `{"status": "stopped"}` response.

## Acceptance Scenarios

1. Start then stop returns one runtime identifier and emits ordered `started`
   then `stopped` lifecycle events.
2. Start then kill emits `started` then `killed`; a repeated kill returns
   `compatible_instance_not_active` and emits nothing.
3. Shutdown with an active instance reports it in
   `killed_compatible_instances` and leaves no active lifecycle state.
4. The shared fixture runs through the same core-Wasm exports consumed by the
   Swift, Kotlin, and .NET host profiles without a sidecar.

## Compatibility

Bridge 1.1 is additive within major version 1. A bridge 1.0 host may ignore the
new exports, but a native package implementing the complete embedder API cannot
certify against a bridge 1.0 runtime.

## Out of Scope

- Platform-specific lifecycle operations or identifiers.
- Compatible-capability business logic in an embedder.
- Changes to the public `embedder-api/1.0.0` operation set.

## Implementation Tickets

- Traverse #647 — Swift/WasmKit adapter.
- Traverse #648 — Kotlin/Chicory adapter.
- Traverse #649 — .NET/Wasmtime adapter.

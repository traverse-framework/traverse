# Feature Specification: Native Runtime-WASM Bridge

**Feature Branch**: `071-native-runtime-wasm-bridge`  
**Created**: 2026-07-15  
**Status**: Approved  
**Version**: 1.0.0  
**Input**: Decision 23, ADR-0007, and Traverse #712.

## Purpose

Define the production, cross-engine bridge by which Swift, Kotlin/Android, and
.NET/WinUI packages host one runtime-owned orchestrator artifact. This is the
native implementation contract beneath `embedder-api/1.0.0`; it does not add
public application operations.

## Artifact and Bundle Contract

- **FR-001**: A native bundle MUST contain `runtime/runtime.wasm`,
  `runtime/runtime-wasm-bridge-1.0.0.json`, the application manifest,
  component manifests, capability modules, and a digest manifest covering
  every file.
- **FR-002**: `runtime.wasm` MUST be a core WebAssembly module implementing
  `runtime-wasm-bridge/1.0.0` and MUST export exactly one bridge memory plus the
  required functions in the governed ABI manifest.
- **FR-003**: Before instantiation, the package MUST verify every declared
  SHA-256 digest, bridge/API compatibility, artifact size, and required engine
  features. A failure MUST prevent instantiation and network/sidecar fallback.
- **FR-004**: Release evidence MUST record package and engine versions,
  licenses, runtime digest, bridge/API/conformance versions, supported hosts,
  and the conformance result.

## ABI and Ownership Contract

- **FR-005**: Inputs are caller-owned UTF-8 JSON bytes at `(pointer, length)`.
  Outputs are runtime-owned UTF-8 JSON referenced by an eight-byte
  little-endian descriptor `{ pointer: u32, length: u32 }` written at a
  caller-owned descriptor address.
- **FR-006**: The module MUST export `traverse_bridge_abi_version`,
  `traverse_alloc`, `traverse_dealloc`, `traverse_init`, `traverse_submit`,
  `traverse_next_event`, `traverse_cancel`, and `traverse_shutdown` with the
  signatures and status codes in the ABI manifest.
- **FR-007**: The host MUST copy an output before its next mutating bridge call.
  The runtime may invalidate prior output regions on such a call. Each input
  allocation MUST be released exactly once by the host.
- **FR-008**: The v1 module MUST NOT require WASI or ambient host capabilities.
  Any permitted import MUST be listed in the bundle manifest and linked by an
  adapter with least privilege.

## Lifecycle, Events, and Errors

- **FR-009**: Calls before successful `init`, duplicate `init`, and calls after
  `shutdown` MUST return deterministic structured errors without trapping.
- **FR-010**: `submit` MUST return the same session identifier and acceptance
  meaning as `embedder-api/1.0.0`; all execution outcomes cross the boundary as
  runtime-owned events.
- **FR-011**: `next_event` MUST return one event at a time in runtime order:
  `1` when an event descriptor is written, `0` when the queue is empty, and a
  negative stable bridge status for failure. Hosts MUST use a single serialized
  drain loop per runtime instance.
- **FR-012**: `cancel` MUST be idempotent. It requests runtime cancellation and
  does not report completion until the runtime emits its terminal event.
- **FR-013**: `shutdown` MUST reject new submissions, cancel active work, drain
  terminal events, release compatible capabilities, and return `stopped`.
- **FR-014**: Expected failures MUST use JSON `{ "code", "message", "details" }`
  with a stable machine code and redacted details. WebAssembly traps, invalid
  descriptors, invalid UTF-8/JSON, and resource exhaustion map to the stable
  host error codes in the ABI manifest.

## Resource and Compatibility Policy

- **FR-015**: The host MUST configure a bounded memory maximum, fuel or an
  equivalent instruction budget, epoch/deadline interruption where supported,
  a maximum event size, and a bounded event-drain queue. Limits and unsupported
  controls MUST be disclosed in release evidence.
- **FR-016**: Compatibility is the intersection of exact bridge major version,
  supported embedder API range, required core-Wasm features, and bundle schema.
  Minor/patch bridge versions may only add optional behavior; incompatible
  major versions fail with `bridge_version_mismatch`.
- **FR-017**: Engine dependencies MUST follow ADR-0007. Exact versions are
  release-pinned and changes require license/security review plus the full
  cross-engine conformance suite.

## Acceptance Scenarios

1. The same fixture bundle initializes through each declared host profile,
   accepts one capability submission, yields ordered `state_changed`,
   `capability_invoked`, and `capability_result` events, then shuts down.
2. A modified runtime or capability digest fails before module instantiation.
3. An incompatible bridge major, malformed descriptor, trap, timeout, or memory
   limit produces the documented stable error without secrets or sidecar use.
4. Swift, Kotlin, and .NET adapters pass both this bridge corpus and the
   `embedder-api/1.0.0` conformance corpus before release.

## Out of Scope

- Public operations beyond `embedder-api/1.0.0`.
- Platform-owned workflow or capability semantics.
- A mandatory Component Model dependency for bridge v1.
- Replacing the development sidecar used by CLI tooling.

## Implementation Tickets

- Traverse #647 — Swift package adapter and WasmKit host.
- Traverse #648 — Kotlin/Android adapter and Chicory host.
- Traverse #649 — .NET/WinUI adapter and Wasmtime host.

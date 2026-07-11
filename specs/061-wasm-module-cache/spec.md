# Feature Specification: WASM Module Cache

**Spec ID**: 061
**Status**: Approved
**Created**: 2026-07-11
**Input**: GitHub issue #585

## Context

Spec `025-wasm-executor-adapter` defines the Wasmtime-backed executor boundary.
This extension defines the compiled-module cache for that executor so repeated
invocations of unchanged WASM bytes reuse Cranelift compilation output.

The cache applies only to compiled `wasmtime::Module` values. Every invocation
still creates a fresh Wasmtime `Store`, WASI context, fuel budget, and store
limits so guest state and resource consumption do not leak between calls.

## Requirements

- **FR-001**: `WasmExecutor` MUST cache compiled `Module` values by the SHA-256
  checksum of the loaded WASM bytes.
- **FR-002**: A repeated execution of identical bytes under the same Traverse
  Host ABI version MUST reuse the cached module instead of recompiling it.
- **FR-003**: The compiled-module cache MUST be bounded by entry count and use a
  deterministic oldest-entry eviction policy.
- **FR-004**: Cache hits MUST NOT bypass checksum verification for
  file-backed execution.
- **FR-005**: Cache hits MUST NOT bypass Host ABI validation; cached entries
  are reusable only for the ABI version they were validated against.
- **FR-006**: WASI context, `Store`, fuel, memory, table, and instance limits
  MUST remain fresh per invocation even when the compiled module is reused.
- **FR-007**: The embedded Host ABI whitelist MUST be parsed once per process,
  not on every module validation.
- **FR-008**: Cache counters MUST expose enough evidence for tests to prove
  first execution misses, repeated execution hits, and eviction is deterministic.

## Out of Scope

- Persisting compiled modules across process restarts.
- Disk-backed Wasmtime engine cache configuration.
- Skipping file reads for checksum-pinned capabilities.
- Runtime HTTP or CLI configuration surfaces for cache sizing.

## Verification

- A test MUST prove a repeated execution of the same WASM bytes records one
  cache miss and one cache hit.
- A test MUST prove cached-module execution still uses a fresh `Store`.
- A test MUST prove bounded cache eviction removes the oldest entry
  deterministically.
- A test MUST prove a cached module does not bypass checksum mismatch detection.
- Existing executor tests, repository checks, Rust checks, coverage gate, and
  spec-alignment checks MUST pass with this spec declared in the PR body.

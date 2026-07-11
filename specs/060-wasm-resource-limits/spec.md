# Feature Specification: WASM Resource Limits

**Spec ID**: 060
**Status**: Approved
**Created**: 2026-07-11
**Input**: GitHub issue #584

## Context

Spec `025-wasm-executor-adapter` defines the Wasmtime-backed executor boundary.
This extension defines the required resource-limit model for that executor so a
guest module cannot run without CPU, memory, table, and instance budgets.

The resource limits apply per invocation. Each invocation already uses a fresh
Wasmtime `Store`; this spec requires that store to receive fresh limits before
the guest entrypoint is called.

## Requirements

- **FR-001**: `WasmExecutor` MUST create its Wasmtime engine with fuel
  consumption enabled.
- **FR-002**: Each WASM invocation MUST set a finite fuel budget before calling
  the guest entrypoint.
- **FR-003**: Fuel exhaustion MUST return a stable timeout-classified executor
  error instead of a generic execution failure.
- **FR-004**: Each WASM invocation MUST apply finite store limits for linear
  memory bytes, table elements, instances, tables, and linear memories.
- **FR-005**: Memory or table growth beyond the configured store limits MUST
  return a stable resource-exhausted executor error instead of a generic
  execution failure.
- **FR-006**: Limits MUST be configurable through the runtime executor API while
  preserving safe defaults for `WasmExecutor::new`.
- **FR-007**: Limits MUST be fresh per invocation so one module cannot consume
  another invocation's budget or mutate future budgets.
- **FR-008**: Existing checksum validation, host ABI validation, deny-by-default
  WASI behavior, and stdout JSON handling MUST keep their existing behavior.

## Out of Scope

- HTTP or CLI configuration surfaces for custom WASM limits.
- Epoch-interruption background tickers.
- Thread-pool task cancellation for native executors.
- Registered-WASM dispatch wiring outside the executor boundary.

## Verification

- A test MUST prove an infinite-loop guest is trapped within a small fuel budget
  and returns the timeout-classified executor error.
- A test MUST prove a guest memory growth beyond the configured cap returns the
  resource-exhausted executor error.
- Existing executor tests MUST continue to pass.
- Repository formatting, Rust checks, and spec-alignment checks MUST pass with
  this spec declared in the PR body.

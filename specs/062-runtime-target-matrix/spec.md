# Feature Specification: Runtime Target Matrix

**Spec ID**: 062
**Status**: Approved
**Created**: 2026-07-11
**Input**: GitHub issue #586

## Context

Traverse needs the core runtime to compile for browser and edge-WASM hosts while
preserving full native execution behavior by default. Native execution adapters
such as Wasmtime and Rayon are valuable on host platforms but are not portable
to `wasm32-unknown-unknown`.

This spec defines the target matrix and feature boundary for the runtime crate.

## Target Matrix

- **Native host**: default Cargo features. Supports runtime orchestration,
  native execution, thread-pool execution, and Wasmtime-backed WASM execution.
- **Browser/edge core**: `wasm32-unknown-unknown` with
  `--no-default-features`. Supports contracts, registry resolution, routing
  types, traces, events, security helpers, and host-provided executors. Excludes
  Wasmtime and Rayon.
- **WASI guest capability**: `wasm32-wasip1` capability crates. These are
  executable artifacts loaded by native `WasmExecutor`, not the host runtime.

## Requirements

- **FR-001**: `traverse-runtime` default features MUST preserve existing native
  behavior, including `WasmExecutor` and `ThreadPoolExecutor`.
- **FR-002**: `wasmtime` and `wasmtime-wasi` MUST be optional behind a
  `wasmtime-executor` feature.
- **FR-003**: `rayon` MUST be optional behind a `native-executors` feature.
- **FR-004**: `traverse-runtime --no-default-features` MUST exclude native-only
  execution dependencies.
- **FR-005**: The no-default core runtime MUST compile for
  `wasm32-unknown-unknown`.
- **FR-006**: CI MUST check the wasm32 no-default runtime build.
- **FR-007**: Documentation MUST state which target and Cargo feature shape
  consumers should use.

## Out of Scope

- Running Wasmtime inside `wasm32-unknown-unknown`.
- Providing a JavaScript host executor implementation.
- Moving native execution adapters into separate crates.
- Changing runtime request, trace, registry, or contract semantics.

## Verification

- `cargo test -p traverse-runtime --test executor_tests -- --nocapture` MUST pass
  with default features.
- `cargo check -p traverse-runtime --no-default-features` MUST pass.
- `cargo check -p traverse-runtime --target wasm32-unknown-unknown
  --no-default-features` MUST pass.
- Repository checks, Rust checks, coverage gate, and spec-alignment checks MUST
  pass with this spec declared in the PR body.

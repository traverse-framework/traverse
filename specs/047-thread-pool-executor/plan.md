# Implementation Plan: ThreadPoolExecutor

**Branch**: `047-thread-pool-executor`
**Date**: 2026-07-03
**Spec**: `specs/047-thread-pool-executor/spec.md`
**Status**: Approved

## Summary

Add a bounded `ThreadPoolExecutor` for native capability execution in `crates/traverse-runtime`. The executor wraps an existing `CapabilityExecutor`, dispatches native calls onto a validated worker pool, catches panics, preserves existing error semantics, and leaves WASM execution untouched.

## Technical Context

- Language: Rust 1.94+
- Workspace: Cargo workspace
- Runtime crate: `crates/traverse-runtime`
- Existing executor contract: `CapabilityExecutor`, `ExecutorCapability`, `ExecutorError`, `ArtifactType`
- Intended dependency: Rayon for the native bounded pool
- WASM constraint: native pool support must not break `traverse-expedition-wasm` or existing WASM executor behavior

## Constitution Check

- Capability-first boundaries: this is a runtime executor adapter, not a new business capability.
- Contracts are source of truth: no capability contract shape changes are introduced.
- Spec alignment: implementation PRs must declare `047-thread-pool-executor`.
- Portability: native pooling must remain isolated from WASM-targeted crates.
- Determinism: output/error semantics must match the existing native executor path for equivalent inputs.
- Quality: no unsafe code, no production unwrap/expect/panic/TODO, and 100% coverage for new executor production lines.

## Project Structure

```text
crates/traverse-runtime/src/executor/
  mod.rs
  thread_pool.rs
crates/traverse-runtime/tests/
  thread_pool_integration.rs
  thread_pool_stress.rs
.github/workflows/
  ci.yml
```

## Implementation Phases

### Phase 1 - Governance

Tracked by #504.

- Add `specs/047-thread-pool-executor/spec.md`.
- Add `specs/047-thread-pool-executor/plan.md`.
- Add approved registry entry in `specs/governance/approved-specs.json`.
- Validate JSON formatting and spec alignment.

### Phase 2 - Core Executor

Tracked by #505.

- Add `crates/traverse-runtime/src/executor/thread_pool.rs`.
- Define `ThreadPoolExecutorConfig`, `ConfigError`, and `ThreadPoolExecutor`.
- Validate capacity range `1..=256`.
- Implement `CapabilityExecutor` for native dispatch.
- Reject `ArtifactType::Wasm` before pool dispatch.
- Catch panics and map to `ExecutorError::ExecutionFailed`.
- Re-export public executor types from `executor/mod.rs`.

### Phase 3 - Unit Coverage

Tracked by #506.

- Add the named unit tests required by #506.
- Prove config validation, success, error propagation, unsupported WASM guard, concurrency, panic recovery, `Send + Sync`, and drop behavior.
- Keep test sleeps bounded and deterministic.
- Maintain 100% line coverage for `thread_pool.rs`.

### Phase 4 - Runtime Integration

Tracked by #507.

- Add `crates/traverse-runtime/tests/thread_pool_integration.rs`.
- Prove router, placement, trace, event broker, workflow, and WASM bypass behavior.
- Verify concurrent execution keeps outputs and traces isolated.

### Phase 5 - Cross-Platform Stress

Tracked by #509.

- Add ignored stress tests in `crates/traverse-runtime/tests/thread_pool_stress.rs`.
- Add required CI stress-test matrix for Linux x86_64, Linux aarch64, macOS x86_64, macOS arm64, and Windows x86_64.
- Include WASM build regression coverage.

## Validation Plan

Governance PR (#504):

```bash
python3 -m json.tool specs/governance/approved-specs.json
BASE_SHA=origin/main HEAD_SHA=HEAD bash scripts/ci/spec_alignment_check.sh <pr-body-file>
bash scripts/ci/repository_checks.sh
git diff --check
```

Implementation and follow-up PRs:

```bash
cargo test
cargo clippy -- -D warnings
bash scripts/ci/repository_checks.sh
bash scripts/ci/coverage_gate.sh
git diff --check
```

Stress PR:

```bash
cargo test -p traverse-runtime --test thread_pool_stress -- --ignored --nocapture
cargo build --target wasm32-wasip1 -p traverse-expedition-wasm
```

## Risks And Mitigations

- **Risk**: Native pooling leaks into WASM builds.
  **Mitigation**: Keep dependency use isolated to runtime native executor paths and add WASM build regression tests.
- **Risk**: Panic handling hides root cause.
  **Mitigation**: Return stable `ExecutorError::ExecutionFailed` with non-sensitive panic classification and preserve trace failure evidence through integration tests.
- **Risk**: Concurrency tests become flaky.
  **Mitigation**: Use bounded timing assertions, deterministic handlers, and stress tests separated from normal unit tests.
- **Risk**: Unbounded resource use.
  **Mitigation**: Validate capacity at construction and cap capacity at 256.

## Dependencies

- #504 must merge before #505-#509.
- #505 must merge before #506.
- #506 must merge before #507.
- #507 must merge before #509.

## Acceptance

The feature is complete only after #504, #505, #506, #507, and #509 are merged and all required CI gates pass.

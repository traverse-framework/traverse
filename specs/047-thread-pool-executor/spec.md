# Feature Specification: ThreadPoolExecutor

**Feature Branch**: `047-thread-pool-executor`
**Created**: 2026-07-03
**Status**: Approved
**Input**: Project 1 issue #504 and downstream implementation issues #505, #506, #507, and #509. Traverse native capability execution currently runs synchronously on the caller thread. The runtime needs a bounded, governed native executor that preserves existing `CapabilityExecutor` behavior while isolating native capability work on worker threads.

## Purpose

This spec defines `ThreadPoolExecutor`, a concrete native `CapabilityExecutor` implementation that dispatches each native capability invocation onto a bounded worker pool.

The executor is a drop-in native execution adapter. It must not change public runtime request shape, placement semantics, registry behavior, trace structure, workflow semantics, or WASM execution. The goal is controlled native concurrency with deterministic error boundaries and no new host coupling in the WASM path.

## User Scenarios and Testing

### User Story 1 - Execute Native Capabilities On A Bounded Pool (Priority: P1)

As a Traverse runtime operator, I want native capability execution to run on a bounded pool so that concurrent native requests do not execute directly on the caller thread.

**Why this priority**: Native execution should be isolated from caller-thread scheduling while preserving the existing `CapabilityExecutor` contract.

**Independent Test**: Execute the same native capability through `NativeExecutor` and `ThreadPoolExecutor` and verify byte-identical JSON output for the same input.

**Acceptance Scenarios**:

1. **Given** a native capability and valid JSON input, **When** it executes through `ThreadPoolExecutor`, **Then** the caller receives the same output the inner executor produced.
2. **Given** the inner executor returns `ExecutorError`, **When** the call runs through the pool, **Then** the same error class is returned to the caller.
3. **Given** the capability artifact type is `Wasm`, **When** `ThreadPoolExecutor` receives it, **Then** it returns `ExecutorError::UnsupportedArtifactType` before dispatching work to the pool.

### User Story 2 - Bound Concurrency With Explicit Configuration (Priority: P1)

As a runtime maintainer, I want pool capacity to be explicit and validated so that invalid or unbounded native execution cannot enter production by accident.

**Why this priority**: A pool executor without strict capacity bounds can create nondeterministic resource pressure.

**Independent Test**: Construct the executor with capacities `0`, `1`, `256`, and `257`, and verify the documented success or `ConfigError::InvalidCapacity` result.

**Acceptance Scenarios**:

1. **Given** capacity `1`, **When** the executor is constructed, **Then** construction succeeds.
2. **Given** capacity `256`, **When** the executor is constructed, **Then** construction succeeds.
3. **Given** capacity `0` or `257`, **When** the executor is constructed, **Then** construction fails with `ConfigError::InvalidCapacity { given, min: 1, max: 256 }`.

### User Story 3 - Preserve Runtime Safety Across Errors And Panics (Priority: P1)

As a runtime maintainer, I want native capability errors and panics to be contained so that one bad native capability call cannot poison the executor.

**Why this priority**: Native extension points must fail predictably and preserve future execution capacity.

**Independent Test**: Run successful, failing, and panicking native handlers through the executor and verify subsequent calls still complete.

**Acceptance Scenarios**:

1. **Given** the inner executor panics, **When** the call runs through `ThreadPoolExecutor`, **Then** the caller receives `ExecutorError::ExecutionFailed` with a message containing `panicked`.
2. **Given** a panicking call has completed, **When** a normal call follows, **Then** the normal call succeeds.
3. **Given** multiple sequential panics occur, **When** a normal call follows, **Then** the pool remains usable.

### User Story 4 - Preserve Full TraverseRuntime Integration (Priority: P2)

As a Traverse runtime integrator, I want `ThreadPoolExecutor` to work through router, placement, workflow, trace, and event broker paths without changing those public surfaces.

**Why this priority**: The executor must be a runtime implementation detail, not a contract break.

**Independent Test**: Wire `ThreadPoolExecutor` through `TraverseRuntime` and run concurrent router, workflow, event, and trace scenarios.

**Acceptance Scenarios**:

1. **Given** concurrent native runtime requests, **When** they execute through the router, **Then** each request receives the correct isolated output.
2. **Given** concurrent executions produce traces, **When** traces are inspected, **Then** trace entries remain isolated by execution id and capability id.
3. **Given** a workflow uses native capability steps, **When** those steps execute through the pool, **Then** workflow traversal completes or fails with the same semantics as the native executor path.

### User Story 5 - Prove Cross-Platform Stability Under Load (Priority: P2)

As a Traverse maintainer, I want stress evidence across supported operating systems so that bounded native concurrency is not only correct on one development host.

**Why this priority**: Threading behavior can vary by OS and architecture.

**Independent Test**: Run ignored stress tests in CI across Linux x86_64, Linux aarch64, macOS x86_64, macOS arm64, and Windows x86_64.

**Acceptance Scenarios**:

1. **Given** sustained concurrent native execution, **When** stress tests run, **Then** all outputs remain correct and no deadlocks occur.
2. **Given** repeated executor create-use-drop cycles, **When** resource checks run, **Then** no file descriptor or handle leak is detected.
3. **Given** the WASM example crate is built after adding thread-pool support, **When** the WASM build runs, **Then** it still succeeds without pulling the native pool into the WASM build path.

## Edge Cases

- Capacity is lower than `1`: reject at construction.
- Capacity is greater than `256`: reject at construction.
- Inner executor returns an error: propagate the error without wrapping it in a success response.
- Inner executor panics: catch the panic and return `ExecutorError::ExecutionFailed`.
- Artifact type is `Wasm`: reject before scheduling work on the pool.
- Multiple calls contend for a capacity-1 pool: serialize without deadlock.
- Many concurrent calls share one executor: outputs must not cross-contaminate inputs or traces.
- Executor is dropped without calls: drop cleanly.
- Executor is dropped after success, error, or panic: drop cleanly.
- WASM build path is evaluated: no native-only pool dependency may leak into WASM crate compilation.

## Requirements

### Functional Requirements

- **FR-001**: Traverse MUST provide a public `ThreadPoolExecutor` type in `crates/traverse-runtime`.
- **FR-002**: Traverse MUST provide `ThreadPoolExecutorConfig { capacity: usize }` with valid range `1..=256`.
- **FR-003**: Traverse MUST provide `ConfigError::InvalidCapacity { given: usize, min: usize, max: usize }` for invalid capacities.
- **FR-004**: `ThreadPoolExecutor::new(config, inner)` MUST validate capacity before constructing the executor.
- **FR-005**: `ThreadPoolExecutor` MUST implement the existing `CapabilityExecutor` trait.
- **FR-006**: `ThreadPoolExecutor` MUST accept an inner `Box<dyn CapabilityExecutor>` and delegate native execution through that executor on the worker pool.
- **FR-007**: `execute()` MUST clone the capability and input required for dispatch and block the caller until the worker returns success or failure.
- **FR-008**: `ArtifactType::Wasm` MUST return `ExecutorError::UnsupportedArtifactType` without touching the worker pool.
- **FR-009**: Panics from the inner executor MUST be caught with `std::panic::catch_unwind(AssertUnwindSafe(...))` and mapped to `ExecutorError::ExecutionFailed`.
- **FR-010**: A worker MUST be returned to available state after success, error, or panic.
- **FR-011**: `ThreadPoolExecutor` MUST be `Send + Sync`.
- **FR-012**: Public executor types MUST be re-exported from `crates/traverse-runtime/src/executor/mod.rs`.
- **FR-013**: Router and runtime integration MUST preserve existing capability selection, placement, trace, workflow, and event semantics.
- **FR-014**: The WASM executor path MUST remain unaffected and MUST NOT route WASM artifacts through `ThreadPoolExecutor`.
- **FR-015**: Cross-platform stress tests MUST run as ignored tests and only execute in the dedicated stress CI job.

### Non-Functional Requirements

- **NFR-001 Determinism**: For the same capability, input, and inner executor behavior, output and error classification MUST match the existing native execution path.
- **NFR-002 Bounded Resources**: Native concurrency MUST be limited by validated pool capacity.
- **NFR-003 Panic Containment**: Panics MUST not poison future executor calls.
- **NFR-004 Portability**: The native pool dependency MUST not break WASM-targeted crates or WASM example builds.
- **NFR-005 Traceability**: Runtime integration tests MUST prove traces remain isolated under concurrent execution.
- **NFR-006 Testability**: Unit, integration, and stress tests MUST cover success, error, panic, concurrency, drop, and WASM-regression paths.
- **NFR-007 Minimality**: This feature MUST add the smallest executor abstraction needed and MUST NOT replace router, workflow, trace, registry, or WASM executor architecture.

### Non-Negotiable Quality Standards

- **QG-001**: No `unsafe` blocks are allowed in the implementation.
- **QG-002**: No production `unwrap()`, `expect()`, `panic!()`, or TODO comments are allowed.
- **QG-003**: Core production lines added for the executor MUST have 100% line coverage.
- **QG-004**: WASM capability execution MUST remain outside the thread-pool path.
- **QG-005**: Panic handling MUST return stable `ExecutorError` evidence and leave the pool usable.
- **QG-006**: Cross-platform stress evidence MUST pass before the feature is considered complete.

### Key Entities

- **ThreadPoolExecutor**: Native executor adapter that dispatches `CapabilityExecutor` calls onto a bounded worker pool.
- **ThreadPoolExecutorConfig**: Configuration object containing validated pool capacity.
- **ConfigError**: Construction-time error for invalid executor configuration.
- **Inner CapabilityExecutor**: Existing executor implementation delegated to by worker threads.
- **Worker Pool**: Bounded execution resource used only for native artifacts.
- **Stress Test Suite**: Ignored test target and CI job proving stability under sustained cross-platform load.

## Success Criteria

- **SC-001**: Native capabilities executed through `ThreadPoolExecutor` return the same success output as the existing native executor path.
- **SC-002**: Invalid pool capacities fail at construction with stable bounds evidence.
- **SC-003**: Native executor errors propagate correctly through the pool.
- **SC-004**: Panicking native handlers return `ExecutorError::ExecutionFailed` and do not prevent later successful calls.
- **SC-005**: Concurrent runtime requests produce isolated outputs and traces.
- **SC-006**: WASM capability execution and WASM crate builds remain unaffected.
- **SC-007**: Dedicated cross-platform stress CI passes on Linux, macOS, and Windows targets.

## Assumptions

- `CapabilityExecutor`, `ExecutorCapability`, `ExecutorError`, and `ArtifactType` remain the public executor contract for this slice.
- Rayon is acceptable as the bounded native worker-pool dependency when introduced by the implementation ticket.
- The executor may block the caller until the worker returns; async runtime integration is out of scope.
- Stress tests may be ignored in normal `cargo test` and run only in the dedicated CI matrix.

## Issue Mapping

- [#504](https://github.com/traverse-framework/Traverse/issues/504) - Approve spec 047-thread-pool-executor into governance.
- [#505](https://github.com/traverse-framework/Traverse/issues/505) - Implement ThreadPoolExecutor core.
- [#506](https://github.com/traverse-framework/Traverse/issues/506) - Unit test suite: ThreadPoolExecutor.
- [#507](https://github.com/traverse-framework/Traverse/issues/507) - Integration test suite: ThreadPoolExecutor through TraverseRuntime.
- [#509](https://github.com/traverse-framework/Traverse/issues/509) - Cross-platform stress test suite: ThreadPoolExecutor.

## Out of Scope

- Changing public runtime request or response schemas.
- Replacing `NativeExecutor`.
- Routing WASM artifacts through the thread pool.
- Introducing Tokio or an async executor surface.
- Changing placement policy, registry semantics, workflow traversal, event broker behavior, or trace schema.
- Adding unbounded native thread creation.

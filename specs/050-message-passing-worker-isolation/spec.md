# Feature Specification: Message-Passing Worker Isolation

**Feature Branch**: `050-message-passing-worker-isolation`
**Created**: 2026-07-03
**Status**: Draft — architecture decision only, no implementation approved
**Input**: `spec 047` delivers thread-pool executor (Option A). This spec captures the Option C architecture so that no decision in 047, the HTTP API, or the MCP surface accidentally closes the door on it. Implementation is gated on v1 shipping and real multi-capability production workloads to design against.

## Purpose

This spec records the intended v2 execution isolation architecture for Traverse: each registered capability type gets a dedicated worker with its own mailbox, lifecycle, and failure boundary. A panicking or hanging capability cannot affect any other capability's execution or the runtime's own state.

This is a **draft spec**. It will not move to `approved` until:
- `spec 047` (thread pool executor) is merged and stable
- At least one real downstream production workload has been observed to need stronger isolation than a shared thread pool provides
- A concrete implementation plan is proposed and reviewed

## Architecture

### Current state (post spec-047)

```
Caller thread
  └── ThreadPoolExecutor
        └── Rayon pool thread
              └── CapabilityExecutor::execute() → Value
```

All capabilities share the same Rayon pool. A slow capability reduces available threads for others. A panicking capability is caught at the pool boundary but the pool thread cycles back into shared use.

### Target state (this spec)

```
Caller thread
  └── WorkerRouter
        └── CapabilityWorker[capability_id]
              ├── inbox: mpsc::Sender<WorkerRequest>
              ├── outbox: oneshot::Sender<WorkerResponse>
              └── dedicated OS thread (or WASM worker)
                    └── CapabilityExecutor::execute() → Value
```

Each capability type has exactly one worker. The worker owns its executor instance. The caller sends a `WorkerRequest` envelope and waits on a `oneshot` channel for the `WorkerResponse`. If the worker thread panics, the worker is restarted and the caller receives `ExecutorError::ExecutionFailed` — no other capability is affected.

### Key properties

| Property | Thread pool (spec 047) | Worker isolation (spec 050) |
|---|---|---|
| Slow capability affects others | Yes (shared pool) | No (dedicated thread) |
| Panicking capability affects pool | No (catch_unwind) | No (worker restarts) |
| Per-capability resource limits | No | Yes (worker owns its resources) |
| WASM worker support | No | Yes (browser WASM workers map directly) |
| Implementation complexity | Low | High |
| API break from spec 047 | — | None (same CapabilityExecutor trait) |

## Deferral Conditions

This spec moves from Draft to Approved only when:

1. A concrete stress test demonstrates that spec-047 thread pool is insufficient for a real workload
2. The implementation plan for worker lifecycle management (start, restart, shutdown, drain) is reviewed and approved
3. WASM worker support on the browser target is validated in `traverse-framework/App-References`

## What Must NOT Be Decided Before This Spec Is Approved

To keep the Option C path open, no other spec may:
- Assume `CapabilityExecutor::execute()` is always called on a pool thread (the trait must remain agnostic)
- Hard-code a Rayon dependency into the public `TraverseRuntime` constructor
- Introduce per-capability mutable state that would be unsafe to move to a dedicated thread

## Files This Spec Will Govern (when approved)

- `crates/traverse-runtime/src/executor/worker.rs` (new)
- `crates/traverse-runtime/src/executor/worker_router.rs` (new)
- `crates/traverse-runtime/src/executor/mod.rs` (re-exports)

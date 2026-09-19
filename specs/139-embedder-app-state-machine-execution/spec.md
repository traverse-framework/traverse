# Feature Specification: Embedder App State Machine Execution

**Feature Branch**: `codex/standalone-embedder-sm-governance`
**Created**: 2026-09-18
**Status**: Approved (2026-09-18)
**Canonical governing ID**: `139-embedder-app-state-machine-execution`
**Version**: 0.1.0
**Extends**: `052-app-state-machine`, `057-embeddable-runtime-host`,
`059-http-command-dispatch`, `068-public-platform-embedder-packages`,
`137-host-connector-command-dispatch`, `1402-runtime-wasm-orchestrator-convergence`
**Amends**: `052-app-state-machine`, `057-embeddable-runtime-host`,
`059-http-command-dispatch`, `1402-runtime-wasm-orchestrator-convergence`,
`embedder-api/1.0.0` → documented additive `1.1.0` submit shapes
**Decision evidence**: Decision 96; ADR-0075 (Accepted).
**Approval**: Owner-approved in `/brainstorm` session 2026-09-18 (Decision 96).

## Purpose and boundary

Close Spec 057 FR-016 for **production embedders**: an application’s
runtime-owned `state_machine` MUST execute inside the application-owned
`runtime.wasm` orchestrator. UI shells render subscribe events and send
commands; they MUST NOT own duplicated business state-machine logic and
MUST NOT require `traverse-cli serve` in production.

This is a **generic platform** capability for every standalone app. Specific
downstream apps (including recording/analysis apps) are acceptance
customers, not the product scope.

## Non-goals (this package)

- New host authorities or browser emulation of native-only connectors
  (Spec 1259 unchanged).
- Durable app state-machine session recovery across process restart
  (follow-up). Stateful *capability* rehydration via host DataStore /
  Spec 1285 remains in force and is a separate layer.
- Replacing Spec 137’s public envelopes or guest `connector_invoke`.

## Relationship to existing specifications

| Specification | Relationship |
| --- | --- |
| 052-app-state-machine | Manifest schema; amended for `invoke.host_connector` and host-connector completion events |
| 057-embeddable-runtime-host | FR-016 becomes executable; submit command envelope documented |
| 059-http-command-dispatch | Semantic parity: same commands/payloads/session rules; HTTP status codes stay HTTP-only |
| 068 / embedder-api | `runtime.submit` accepts workflow/capability **or** app-command envelope |
| 137-host-connector-command-dispatch | Host-connector invoke from SM wait via bridge; adapters unchanged |
| 1402 | `runtime.wasm` owns SM orchestration; nested wasmi memory ceiling 32 MiB |
| 1285 / 085 | Stateful capability partitions rehydrate independently of SM sessions |

## Architecture

```text
UI shell  --submit(app_command)-->  Platform embedder
                                      |  runtime-wasm bridge
                                      v
                              runtime.wasm (Process Manager)
                                 |                |
                    nested capability      host-connector request
                    (wasmi, 32 MiB)        + deadline registration
                                                 |
                                                 v
                                           Host adapter
                                    (Spec 137 dispatch / timer)
```

## Functional requirements

### Command surface

- **FR-001**: Platform embedders MUST accept app state-machine commands
  through existing `runtime.submit` using a typed app-command envelope
  discriminated fail-closed from workflow/capability target submit.
- **FR-002**: The app-command envelope MUST carry at least `kind` =
  `app_command`, `command` (state-machine `on` value), `payload` object,
  and optional `session_id`. Ambiguous submits MUST reject with a stable
  embedder/runtime error code.
- **FR-003**: Command names, payload mapping (`invoke.input_from:
  command.payload`), and session identity rules MUST be semantically
  identical to Spec 059 HTTP command dispatch. HTTP status codes (202 /
  409 / 422 / 404) MUST NOT appear as required embedder fields.
- **FR-004**: `runtime.subscribe` MUST deliver state and outcome events
  with the same JSON event shapes Spec 033/059 clients already consume
  where applicable (`state_changed`, invoke/connector outcomes, `error`).

### Execution locus

- **FR-005**: The application `state_machine` MUST execute inside
  `runtime.wasm`. Platform embedders MUST NOT re-implement transition
  tables, output field computation, or business recovery logic.
- **FR-006**: `traverse-cli serve` MUST share the same transition / saga
  semantics as the embedder path (transport differs only).

### Process Manager / Saga waits

- **FR-007**: A state MAY declare at most one of `invoke.capability_id` or
  `invoke.host_connector` (mutually exclusive).
- **FR-008**: Entering an invoke MUST place the session in an explicit wait
  correlated by `command_id` and `correlation_id` until a terminal event.
- **FR-009**: Capability invoke completion events remain
  `capability_succeeded` and `capability_failed` (existing vocabulary).
- **FR-010**: Host-connector invoke completion events MUST be exactly:
  `host_connector_succeeded`, `host_connector_failed`,
  `host_connector_cancelled`, `host_connector_timeout`.
- **FR-011**: `invoke.host_connector` MUST resolve through Spec 137
  (`dispatch_host_connector_command`) via a runtime-wasm **bridge
  host-import**. Guests MUST NOT gain microphone/model authority; Spec 104
  `connector_invoke` and Spec 135 WIT fakes are out of this surface.
- **FR-012**: Dual deadlines MUST apply to host-connector waits: the host
  MAY complete earlier; `runtime.wasm` MUST register a hard ceiling through
  a host-provided monotonic timer callback. The first correlated terminal
  event wins; later duplicates MUST be ignored idempotently.
- **FR-013**: Fuel/instruction budgets MUST NOT be used as the I/O wait
  timeout mechanism for host-connector sagas.

### Validation and fail-closed recovery

- **FR-014**: `traverse-cli app validate` MUST reject manifests whose
  invoke waits omit required unhappy routes: failure and timeout for every
  invoke; cancel when the operation is cancellable.
- **FR-015**: If a correlated terminal event arrives with no matching `on`
  transition, the runtime MUST fail closed: emit a deterministic session
  `error` (stable reason code) and leave no silent hang. It MUST NOT push
  recovery ownership to the UI.

### Session durability boundary

- **FR-016**: App state-machine sessions in this slice are **process-local**
  to the embedder / `runtime.wasm` lifetime. Restart starts a new session.
- **FR-017**: Stateful capability partition data MUST continue to rehydrate
  through host-owned DataStore / Spec 1285 independently of FR-016. Specs
  and docs MUST NOT imply that Stateful rehydration restores SM session
  wait state.

### Nested memory (orchestrator)

- **FR-018**: Nested capability execution inside `runtime.wasm` MUST default
  to a **32 MiB** linear-memory ceiling (matching native `WasmExecutor` /
  `#1336`) so certified registry planners that reserve ~17 MiB initial
  memory can instantiate. Changing the ceiling requires rebuilding and
  re-certifying the published `runtime.wasm` digest.

## Acceptance scenarios

1. Given a bundled app with `state_machine` and no sidecar, when the UI
   submits an `app_command` envelope, then the session transitions and
   subscribe events reflect runtime-owned state only.
2. Given a state with `invoke.host_connector` for `capture_audio`, when the
   host returns Spec 137 success, then the SM advances on
   `host_connector_succeeded` with an opaque `artifact_ref`.
3. Given the same wait, when the host stays silent past the runtime hard
   ceiling, then the SM advances on `host_connector_timeout` (or fail-closes
   per FR-015 if undeclared).
4. Given a state with `invoke.capability_id`, when nested instantiation
   would previously die at 16 MiB on a ~17.8 MiB planner, then execution
   proceeds under the 32 MiB ceiling.
5. Given HTTP `serve` and an embedder loading the same manifest, when the
   same command sequence is applied, then transition outcomes match
   semantically.
6. Given process restart, when Stateful capability data exists in the host
   store, then capability state rehydrates while the app SM session does
   not silently resume a prior wait.

## Out of scope

- Browser activation of native-only `traverse.audio-input` (remains
  `target_incompatible` per Spec 1259).
- Durable SM session journal / crash recovery.
- New embedder-api operations beyond documenting command envelopes on
  `runtime.submit`.
- Swift WasmKit removal packaging (implementation under Specs 074/076).

## Governance

- ADR-0075 records the Process Manager, dual-deadline, timer-port, and
  submit-envelope decisions.
- Implementation tickets MUST NOT be `Ready` until this spec and ADR-0075
  are approved (this document + Decision 96).

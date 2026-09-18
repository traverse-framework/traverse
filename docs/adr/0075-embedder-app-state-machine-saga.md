# ADR-0075: Embedder App State Machine as Process Manager with Dual Deadlines

- Status: Accepted
- Date: 2026-09-18
- Governing spec: `139-embedder-app-state-machine-execution` (Approved)
- Related decisions: Decision 96
- Extends: ADR-0071; ADR-0072; Specs `052`, `057`, `059`, `137`, `1402`

## Context

Standalone apps must run a runtime-owned application `state_machine` inside
the embedded `runtime.wasm` orchestrator (Spec 057 FR-016) without a
`traverse-cli serve` sidecar. Spec 137 already defines host-connector
command dispatch for host-owned authorities (for example `audio.capture`),
but emitting a connector request from an event-driven machine creates the
classic unanswered-emit problem: deny, cancel, adapter crash, or silence
can hang or leak recovery into the UI.

Stateful capability partitions (Spec 1285 / host DataStore) already
rehydrate across process restart. App state-machine *sessions* are a
different layer and were still missing an embedder execution path.

## Decision

1. **Execute the app state machine inside `runtime.wasm`**, not in each
   platform embedder host and not only in `serve`.
2. **Treat the app state machine as a Process Manager / Saga** for host
   work: entering an invoke wait is correlated by `command_id` /
   `correlation_id`, with declared transitions for success, typed failure,
   cancel, and timeout.
3. **Reach Spec 137 through a bridge host-import**: `runtime.wasm` emits a
   typed host-connector request; the embedder adapter runs
   `dispatch_host_connector_command` / platform driver and returns a
   correlated terminal result or event. Authority stays host-owned.
4. **Use dual deadlines**: the host may complete earlier with success,
   typed failure, cancel, or host timeout; `runtime.wasm` registers a
   hard ceiling via a **host-provided monotonic timer callback**. The first
   correlated terminal event wins; later duplicates are ignored
   (idempotent).
5. **Extend `runtime.submit`** with a typed app-command envelope
   (discriminator fail-closed), preserving Spec 059’s promise of same
   command names/payloads as HTTP. Do not add a parallel `runtime.command`
   verb in this cut.
6. **Semantic parity with `serve`**: transition meaning, session rules, and
   event shapes match; HTTP status codes remain transport-only.
7. **Process-local SM sessions this package**; Stateful capability
   rehydration remains host DataStore–owned and separate. Durable SM
   sessions are an explicit follow-up.
8. **Missing unhappy routes**: `app validate` requires declared success +
   failure + timeout (+ cancel when cancellable) for every invoke wait;
   runtime still fail-closes to a deterministic session error if a
   terminal event has no matching `on` transition.
9. **Manifest invoke forms**: keep `invoke.capability_id`; add mutually
   exclusive `invoke.host_connector`. Completion events are distinct:
   `capability_*` vs `host_connector_succeeded|failed|cancelled|timeout`.
10. **Nested wasmi memory** defaults to **32 MiB**, matching native
    `WasmExecutor` / issue `#1336`, with certified `runtime.wasm` rebuild.

## Consequences

- Spec 139 governs the embedder/`runtime.wasm` command + saga path.
- Specs 052 / 057 / 059 / 1402 and embedder-api gain surgical amendments.
- Swift packaging cleanup (wasmi-only, remove WasmKit production dep) is
  implementation under existing Specs 074/076 — no new product spec.
- Browser/native authority rules from Spec 1259 are unchanged this package
  (native-only audio remains native-only).

## Alternatives considered

- Per-embedder SM hosts: rejected; multi-host drift vs Spec 057.
- Embedder pre-dispatch of connectors outside the SM: rejected; splits
  ownership after choosing runtime-owned SM.
- Request/reply without timeout states: rejected; silent host hang remains.
- Transactional outbox durability in this cut: rejected; permanence scope.
- Fuel-as-timeout or wall clock inside `runtime.wasm`: rejected; wrong
  layer for I/O waits / determinism.
- New `runtime.command` operation: rejected; Spec 059 already named
  `submit`.
- Reusing `capability_*` events for connectors: rejected; authority lie.
- Coupling SM resume to Stateful rehydration: rejected; different layers.

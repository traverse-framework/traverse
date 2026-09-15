# ADR-0072: Converge Browser and Native Embedders on a Real `runtime.wasm` Orchestrator

- Status: Accepted
- Date: 2026-09-14
- Governing specs: `1402-runtime-wasm-orchestrator-convergence` (new),
  `071-native-runtime-wasm-bridge`, `068-public-platform-embedder-packages`,
  `098-capability-event-host-abi`, `1277-browser-local-workflow-composition`
- Related issues: `#1402`, `#1403`, `#1404`
- Related: `docs/decision-log.md` Decision 86
- Owner: Traverse maintainers

## Context

An investigation into why `traverse_host::emit_event` is a no-op stub in the
browser embedder (`packages/web/TraverseEmbedder`) — silently dropping every
capability-emitted business event, surfaced by a `/discover` demo executing a
real ~31MB ML-model capability and expecting progress events — found the gap
is architectural, not a one-line omission:

1. `BundleEmbedder` (the browser package's production execution path)
   reimplements a thin per-capability WASM executor directly in TypeScript
   (`WebAssembly.compile`/`instantiate` plus a hand-written WASI preview1
   shim), rather than loading and driving a `runtime.wasm` orchestrator the
   way `071-native-runtime-wasm-bridge` describes for Swift/Kotlin/.NET. Its
   own code comment justifies this via spec `068` FR-002 ("no nested WASM
   engine"), but Decision 18 (the decision underlying spec `068`) explicitly
   says platform packages "load application-owned **runtime and capability**
   bundles" — for all five platforms, browser included — and FR-002's literal
   text lists "the runtime WASM" as required bundle content. The browser
   package's reading looks like a misapplication of the spec it cites, not a
   separately governed divergence.
2. Because it reimplements the executor, it also had to reimplement
   `traverse_host::emit_event`'s validation logic (bounds check, `emits`-list
   check, `service_type` check) rather than reusing
   `crates/traverse-runtime/src/executor/wasm.rs`'s `handle_emit_event` — and
   that reimplementation was never written, hence the `-1` stub.
3. Investigating the "just adopt `runtime.wasm`" alternative found
   `runtime.wasm` itself, as built by `crates/traverse-native-bridge`, is a
   hand-authored WAT fixture returning hardcoded canned JSON for a fixed
   3-event sequence — no real capability execution, no `EventBroker`, no
   `emit_event` validation exist inside it today, for *any* platform. Native
   embedders' conformance work to date (ADR-0014, ADR-0070, spec `074`, `075`,
   `136`) has been real and is actively shipping, but it certifies the *host
   bridge engine* (wasmi/Chicory/Wasmtime hosting the artifact), not the
   artifact's content.
4. Nothing in this codebase addresses running a capability WASM module from
   *inside* `runtime.wasm` itself (WASM-hosting-WASM) — the native host
   engines solve running `runtime.wasm`, not what `runtime.wasm` would need
   to do internally to execute a second, different WASM module. This is an
   open feasibility question, not a solved one.

This was worked through as an owner-directed `/brainstorm` session (Decision
86). The owner explicitly chose the larger convergence direction over a
narrower browser-only patch, after each of these findings was surfaced in
turn.

## Decision

1. **Build a real `runtime.wasm`.** Replace `crates/traverse-native-bridge`'s
   WAT fixture with an actual `wasm32` build of `traverse-runtime`'s
   `PlacementRouter`, `EventBroker`, and `WasmExecutor` logic (or the design a
   feasibility spike recommends — see below), exporting the unchanged
   `runtime-wasm-bridge/1.0.0` ABI so existing native adapters need no
   changes.
2. **Prove nested execution before committing to its shape.** A feasibility
   spike (`#1403`) must demonstrate a capability WASM module executing from
   inside a `wasm32`-compiled orchestrator — most plausibly via a pure-Rust,
   no-JIT interpreter such as `wasmi` (already production-vetted for the
   Swift *host* profile, ADR-0070, though never applied to this *embedded*
   role) — before Phase 2 implementation work begins. If the spike finds
   nested-interpreter execution impractical, this ADR's Phase 2/3 scope is
   amended to the recommended alternative rather than proceeding on an
   unproven premise.
3. **Converge browser onto the same artifact.** Once the real orchestrator
   exists and passes conformance, `packages/web/TraverseEmbedder` adopts it
   (loaded from the application-owned bundle per `068` FR-002's literal
   text, not embedded in the npm package) and retires `BundleEmbedder`'s
   hand-rolled executor entirely, including `composedWorkflow.ts`'s spec-1277
   composed-execution path — preserving that path's existing security
   properties (host-owned verified cache, no loader/fetcher) unchanged.
4. **Ship an interim, explicitly temporary patch now.** Because the
   convergence above is a multi-phase, multi-month initiative with a real
   open feasibility question, and the reported bug (dropped events in the
   `/discover` demo) is live today, `#1404` authorizes implementing
   `emit_event`'s validation directly in TypeScript in the meantime, under
   the already-approved `098-capability-event-host-abi` (no new governance
   needed for that piece — it is a conformance fix against an existing
   contract, not a new architectural decision). It is marked temporary in
   code and removed once Phase 3 lands.

## Consequences

- `#1403` (feasibility spike) becomes the practical gating item for all of
  Phase 2 and Phase 3; nothing past it should be treated as committed in
  detail until it reports.
- `#1404` ships independently and fixes the actually-reported problem this
  cycle, without waiting on the larger initiative.
- Native embedder work already in flight (wasmi 2.0.0 host engine, XCFramework
  publication, spec `136`) is unaffected — this ADR changes what
  `runtime.wasm` contains, not how it is hosted or distributed.
- `traverse-embedder-web` will eventually require a major-version bump when
  `BundleEmbedder`'s executor is retired (spec `1402` FR-010), since it is a
  published, production-consumed package.

## Alternatives Considered

- **Patch the browser in place, keep two architectures indefinitely
  (Option A from the brainstorm).** Rejected by the owner: smaller and safer
  short-term, but leaves the underlying logic-duplication risk (of which this
  `emit_event` gap is the first concrete symptom) permanently unaddressed —
  every future `traverse_host` function would need the same manual
  reimplementation in TypeScript.
- **Adopt `runtime.wasm` for browser only, leave native on its fixture.**
  Rejected: native's fixture is the same underlying problem, just not yet
  reported as a bug; fixing it only for browser would leave native's
  conformance tests validating bridge plumbing against fake data
  indefinitely.
- **Skip the feasibility spike and commit directly to a nested-interpreter
  design.** Rejected: WASM-hosting-WASM is genuinely unattempted in this
  codebase; committing FRs to an unproven mechanism risks the same kind of
  spec-vs-reality drift this investigation just found in `068` FR-002.

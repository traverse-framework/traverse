# Feature Specification: Runtime-WASM Orchestrator Convergence (Browser + Native)

**Status**: Approved
**Canonical governing ID**: `1402-runtime-wasm-orchestrator-convergence`
**Version**: 1.1.0
**Extends**: `071-native-runtime-wasm-bridge`, `068-public-platform-embedder-packages`,
`098-capability-event-host-abi`, `995-local-executor-event-emission`
**Amends**: `1277-browser-local-workflow-composition` (its composed-execution
mechanism migrates under this spec's Phase 3; its security model does not
change)
**Input**: Issue #1402; ADR-0072; `/brainstorm` session recorded as Decision 86
in `docs/decision-log.md`.

**Amendment (2026-09-15, version 1.0.0 -> 1.1.0, approved 2026-09-15)**: FR-002's
gate resolved — Decision 87 (spike `#1403`, ADR-0072) recorded nested wasmi as the
proven design, which is a different dispatch design than FR-005's original text
assumed. Implementing #1407 (Phase 2) surfaced the concrete conflict: `WasmExecutor`
(`crates/traverse-runtime/src/executor/wasm.rs`) is built on Wasmtime, which cannot
itself target `wasm32` — so the nested-wasmi executor inside `runtime.wasm` cannot
literally be "the same compiled code path" FR-005 originally required. Resolved via
a live, owner-participated brainstorm (2026-09-15, recorded as Decision 88 in
`docs/decision-log.md`): FR-005 changes from "same compiled code path" to "same
shared, engine-agnostic validation core," and a new FR-011 governs the new crate
and audited `unsafe_code` boundary the real ABI export requires. FR-003, FR-004,
Acceptance Scenario 2, and the Capability Boundary section are updated to match.
No other requirement changed.

## Purpose

`runtime.wasm` (built by `crates/traverse-native-bridge`) is a hand-authored
WAT fixture that returns hardcoded canned JSON for a fixed 3-event sequence.
It contains no real capability execution, no `EventBroker`, and no
`traverse_host::emit_event` validation. Swift, Kotlin, and .NET embedders
(`071-native-runtime-wasm-bridge`) exercise their bridge *adapters*
(wasmi/Chicory/Wasmtime host engines) against this fixture today, not against
real runtime logic. The browser embedder (`traverse-embedder-web`) never
adopted `runtime.wasm` at all: `BundleEmbedder` and `composedWorkflow.ts`
reimplement a thin per-capability executor directly in TypeScript, which is
why `traverse_host::emit_event` is a hardcoded no-op stub there and every
capability-declared business event is silently dropped in the browser (the
gap that surfaced this investigation, via the `/discover` demo).

Decision 18 (`docs/decision-log.md`, governing `068`) originally intended all
five platforms to "load application-owned runtime and capability bundles."
The browser package's divergence from that — reimplementing executor logic
by hand instead of loading a runtime artifact — was not a separately governed
decision; it reads as a misapplication of `068` FR-002.

This spec governs the phased plan to: build a real `runtime.wasm` containing
the actual `traverse-runtime` execution and event logic; prove that a
capability WASM module can be executed from inside it (WASM-hosting-WASM, an
unsolved problem in this codebase today); and converge every embedder —
native and browser alike — onto that one artifact, retiring both the native
fixture and the browser's hand-rolled executor.

## Capability Boundary

Governs: `crates/traverse-native-bridge` (the `runtime.wasm` builder), a new
`crates/traverse-runtime-wasm` crate (the `wasm32`-target nested-wasmi
executor and C-ABI export boundary, FR-011), a shared engine-agnostic
`emit_event` validation core in `crates/traverse-contracts` (FR-005),
`packages/web/TraverseEmbedder`'s `BundleEmbedder` and `composedWorkflow.ts`
(their eventual retirement/replacement), and how native packages consume the
resulting artifact.

Does not govern: the native host bridge engine selection itself
(wasmi/Wasmtime/Chicory — ADR-0014/ADR-0070, unchanged by this spec); the
`runtime-wasm-bridge/1.0.0` ABI surface itself (`071`, unchanged — the new
`runtime.wasm` must keep exporting the same functions); the interim
TypeScript `emit_event` implementation in the browser package (governed by
the already-approved `098-capability-event-host-abi`, tracked as issue
`#1404`, explicitly temporary and superseded once this spec's Phase 3
lands); `connector_invoke`'s behavior in any embedder (a separate, unaddressed
question, out of scope here as it was for the originating investigation).

## Requirements

### Phase 1 — Feasibility (tracked as issue #1403)

- **FR-001**: A spike MUST demonstrate a capability WASM module executing
  from inside a `wasm32`-compiled orchestrator module running under a normal
  WASM host, before Phase 2 begins.
- **FR-002**: The spike's findings — a recommendation to proceed with
  nested-interpreter execution, or an alternative capability-dispatch design
  — MUST be recorded as a decision-log entry before any Phase 2 requirement
  below is treated as final. If the recommended design differs materially
  from nested-interpreter execution, this spec MUST be amended to reflect it
  before Phase 2 work begins (this codebase's immutable-spec-amendment
  process, not a silent reinterpretation).

### Phase 2 — Real orchestrator content

- **FR-003**: `crates/traverse-native-bridge`'s WAT fixture MUST be replaced
  by a real `wasm32` build implementing `traverse-runtime`'s
  `PlacementRouter`, `EventBroker`, and capability-dispatch logic per the
  nested-wasmi design Decision 87 recommends, exporting the unchanged
  `runtime-wasm-bridge/1.0.0` ABI (`071`) so existing native host adapters
  require no changes. Phase 2 MAY land incrementally across multiple PRs; an
  intermediate PR's reduced scope (e.g. a minimal `PlacementRouter`/
  `EventBroker` slice, a narrower conformance test) is not itself a spec
  violation as long as FR-004's full bar is met before Phase 2 is tagged
  complete and before any release.
- **FR-004**: The rebuilt `runtime.wasm` MUST pass the existing bridge and
  embedder conformance corpora (`071` Acceptance Scenario 4, `068` FR-009)
  before any release. This applies to Phase 2's completion/release gate, not
  to every intermediate PR (see FR-003).
- **FR-005**: `traverse_host::emit_event`'s validation behavior inside the
  new `runtime.wasm` MUST be identical to
  `crates/traverse-runtime/src/executor/wasm.rs`'s `handle_emit_event`
  (`098` FR-002/FR-003/FR-008) — same acceptance criteria, same error codes —
  because both call the same shared, engine-agnostic validation core (see
  FR-011), not two independently maintained implementations. The core
  operates on plain byte slices and the capability's declared `EventReference`
  list; it does not depend on which engine (Wasmtime, natively, or wasmi,
  nested) read the guest's linear memory.
- **FR-011**: The nested-wasmi executor and its C-ABI export boundary
  (reading/writing the guest's own linear memory for `traverse_init`,
  `traverse_submit`, `traverse_next_event`, and the rest of `071` FR-006's
  export list) MUST live in a new, dedicated crate
  (`crates/traverse-runtime-wasm`) rather than extending
  `crates/traverse-native-bridge` (the native-side WAT/artifact builder,
  unrelated in kind) or `crates/traverse-nested-wasm-spike` (explicitly
  spike-scoped, `#![deny(unsafe_code)]`, kept as historical record). This new
  crate MAY declare `#![allow(unsafe_code)]`, scoped to the minimum surface
  the ptr/len-to-slice ABI conversions require, under a new audited exception
  (ADR-0073) following the precedent `076-production-swift-wasmi-cabi` set
  for `traverse-swift-host`. The shared validation core (FR-005) MUST NOT
  itself require `unsafe_code` and MUST live in `crates/traverse-contracts`
  (already the home of `EventReference`), built as an ordinary `std` crate —
  `wasm32-unknown-unknown` supports `std`; only `crates/traverse-nested-wasm-
  spike`'s own `wasmi` dependency configuration chose `no_std` + `alloc`; that
  choice does not propagate to crates that merely depend on it.

### Phase 3 — Browser convergence

- **FR-006**: `packages/web/TraverseEmbedder` MUST offer a
  `runtime.wasm`-backed implementation of `TraverseEmbedderApi` (`057`
  FR-003; `068`'s uniform embedder surface), loading `runtime.wasm` from the
  application-owned bundle per `068` FR-002's literal text — not embedded in
  the npm package — using the same `BundleLoader` abstraction already used
  for capability artifacts.
- **FR-007**: Once FR-006 ships and passes conformance, `BundleEmbedder`'s
  hand-rolled per-capability `WebAssembly.compile`/`instantiate` executor
  (`bundleEmbedder.ts`, `wasi.ts`, and `hostAbi.ts`'s per-capability import
  validation) MUST be retired — not kept as a second, permanently supported
  execution path (matching this codebase's established no-back-compat-tax
  pattern, e.g. `098` FR-004).
- **FR-008**: `1277-browser-local-workflow-composition`'s offline
  composed-execution path (`composedWorkflow.ts`) MUST migrate to the same
  `runtime.wasm`-backed execution, preserving its existing security
  properties unchanged — host-owned verified cache only, no loader/fetcher,
  snapshot-digest binding to the reviewed proposal. This spec changes what
  executes the verified artifacts, not `1277`'s trust boundary.
- **FR-009**: The interim TypeScript `emit_event` implementation (`#1404`)
  MUST be removed once FR-007 completes; it is explicitly temporary and MUST
  be marked as such in its own code comments until then.
- **FR-010**: Migration MUST NOT break existing published
  `traverse-embedder-web` consumers without a documented major-version bump
  and migration note, given the package is already in production use
  (`1277`, npm `0.9.0+`).

## Acceptance Scenarios

1. Given the Phase 1 spike concludes with a workable nested-execution design,
   when Phase 2 begins, then FR-003 through FR-005 apply as amended (Decision
   88); the spike did recommend a different dispatch design than originally
   assumed (nested wasmi, Decision 87), and this spec was amended accordingly
   before Phase 2 FRs were treated as binding, per this scenario's own
   condition.
2. Given a `Subscribable` capability declares an event in its `emits` list
   and calls `emit_event` while running inside the Phase-2 `runtime.wasm` on
   any platform, native or browser, when execution completes, then the event
   reaches `EventBroker` with identical validation behavior across every
   platform, because every platform calls the same shared validation core
   (FR-005, FR-011) — not because it is the same compiled binary, which is
   no longer true once native (Wasmtime) and nested (wasmi) engines differ.
3. Given `BundleEmbedder`'s hand-rolled executor is retired (FR-007), when an
   existing bundled app upgrades to the new major version, then its
   capability and workflow execution produces identical outputs, verified by
   running the existing conformance corpus (`068` FR-009) against both the
   old and new executor over the same fixture set during the migration
   window.
4. Given spec `1277`'s composed-execution path migrates (FR-008), when a
   reviewed workflow proposal executes, then it still refuses to run
   anything outside the host-owned verified cache, exactly as today.
5. Given the interim patch (`#1404`) is live and FR-007 has not yet
   completed, when a developer inspects `composedWorkflow.ts` or
   `bundleEmbedder.ts`, then a code comment identifies the `emit_event`
   implementation there as temporary and references this spec.

## Out of Scope

- The native host bridge engine selection itself (`wasmi` 2.0.0, etc.) —
  unchanged, governed by ADR-0070.
- `connector_invoke`'s stub behavior in the browser embedder — a separate
  question, not addressed here.
- The interim browser `emit_event` patch's own implementation details —
  governed by the already-approved `098-capability-event-host-abi` via issue
  `#1404`, referenced here only as the thing Phase 3 retires.
- A mandatory Component Model dependency (matching `071`'s existing scope
  cut).
- Performance tuning of the nested-execution design beyond FR-002's rough
  spike-stage measurement.

## Governing Relationship

This specification extends `071-native-runtime-wasm-bridge`,
`068-public-platform-embedder-packages`, `098-capability-event-host-abi`, and
`995-local-executor-event-emission`. It amends
`1277-browser-local-workflow-composition`'s execution mechanism (not its
security model) once Phase 3 completes.

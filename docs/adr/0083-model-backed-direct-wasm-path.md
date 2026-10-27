# ADR-0083: Route Model-Backed Capabilities to Direct WebAssembly (Not Nested wasmi)

- Status: Accepted
- Date: 2026-10-27
- Governing specs: `138-governed-exact-model-execution`,
  `1402-runtime-wasm-orchestrator-convergence`
- Related: Decision 117; ADR-0074; ADR-0072; ADR-0073; Spec 068
- Issues: `#1657` (nested wasmi ~50–80× slower for model math; sync submit
  freezes the browser tab)
- Approval: Owner-directed implementation path for `#1657` (2026-10-27)

## Context

Nested capability execution inside `runtime.wasm` (wasmi interpreter hosted
in a WebAssembly module) is the governed Browser/Node path for ordinary
bundled capabilities (Spec 1402). Prototype measurements for Hugging Face
catalog agents showed that path is **~50–80× slower than native wasmtime**
for DistilBERT-NER and Silero VAD, and Whisper transcription did not finish
within 25 minutes on Node. See
[`docs/perf/nested-vs-direct-wasm-1657.md`](../perf/nested-vs-direct-wasm-1657.md).

Separately, Spec 138 already ships a **direct WebAssembly** host path:
`ExactModelBrowserHost` instantiates model guests with the browser/Node
`WebAssembly` engine (not nested wasmi). ONNX MNIST-class packages complete
in tens to low hundreds of milliseconds with byte-identical outputs vs
native.

Making nested wasmi 50–80× faster while preserving fuel metering and
determinism is not a short-term product path. Browser tabs also freeze when
`BundleEmbedder.submit` runs long nested work synchronously.

## Decision

1. **Model-backed product work on Browser/Node uses the Spec 138 direct
   WebAssembly path** (`ExactModelBrowserHost` / native `WebAssembly`
   instantiate), not nested wasmi inside `runtime.wasm`. Catalog HF agents
   that today ship as plain nested capabilities must be packaged (or
   re-packaged) as Spec 138 exact-ref model packages when they need the
   ≤10× native latency bound on Node/Chrome.
2. **Nested wasmi remains the governed path for non-model bundled
   capabilities** (small WASM business logic, workflow orchestration,
   Spec 139 app commands). Spec 1402 and ADR-0073 are unchanged.
3. **`traverse-embedder-web` adds `submitAsync`** with optional
   `AbortSignal`. Cancel returns `status: "cancelled"` and records Trace
   API phase/outcome `cancelled`. Sync `submit` stays for small nested
   work. Mid-flight interrupt of a single nested wasmi invoke still
   requires Worker isolation (documented); cooperative cancel checks run
   before/after nested invoke and between workflow nodes.
4. **Acceptance bound for `#1657`:** model-backed capabilities meet the
   ≤10× native overhead bar on the Spec 138 path (evidenced by checked-in
   lightweight fixture timings plus the documented HF bench procedure),
   not by tuning nested wasmi to that bound.

## Consequences

- Product demos (redact, voice-memo, NER, VAD, transcription) should pin
  Spec 138 packages and invoke through `ExactModelBrowserHost` /
  `model.execute`, not `BundleEmbedder.submit` of a nested HF WASM guest.
- Nested fuel/memory raises (#1656) remain useful for large non-model or
  transitional nested guests, but are not the latency fix for model math.
- Async submit unblocks UI responsiveness for remaining nested work;
  Worker offload is guidance, not a second embedder API.

## Alternatives considered

- **Tune nested wasmi (coarser fuel, SIMD, upgrade) until ≤10×.** Rejected
  as the primary fix: the gap is interpreter-in-interpreter math, not a
  small constant factor; preserving Spec 1402 metering while closing a
  50–80× gap is not evidenced.
- **Replace all nested execution with direct WebAssembly.** Rejected:
  orchestration, emit validation, and fuel limits for ordinary
  capabilities still belong in `runtime.wasm`.
- **Only document Worker wrapping of sync `submit`.** Rejected: hosts
  still need a Promise + AbortSignal surface and cancelled Trace
  evidence.

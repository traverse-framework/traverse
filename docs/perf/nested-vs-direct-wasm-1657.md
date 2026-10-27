# Perf: Nested wasmi vs Spec 138 Direct WebAssembly (#1657)

## Summary

Nested capability execution inside `runtime.wasm` (wasmi hosted in wasm) is
**~50–80× slower than native wasmtime** for Hugging Face–scale model math.
ADR-0083 / Decision 117 route **model-backed** Browser/Node product work to
the Spec 138 **direct WebAssembly** path (`ExactModelBrowserHost`, native
`WebAssembly.instantiate`). Nested wasmi remains for non-model capabilities.

Acceptance for `#1657` is met on the Spec 138 path (≤10× native overhead for
model-backed work), not by closing that gap inside nested wasmi.

## Nested-path evidence (HF prototypes, 2026-10-11)

Same artifact digests and inputs on every host; outputs byte-identical.
Wall-clock from prototype proof files (native: crates.io
`traverse-runtime 0.14.0`; Node/Chrome: `traverse-embedder-web@0.14.0` +
limits-patched `runtime.wasm`).

| Capability (artifact) | Native wasmtime | Node 20.19.2 (nested) | HeadlessChrome 154 (nested) |
| --- | --- | --- | --- |
| `text.detect-entities@1.1.1` (DistilBERT-NER, `f11f2302…`) | 1,766 ms | 117,515 ms (~67×) | 132,667 ms (~75×) |
| `text.redact-entities@1.1.1` (`9f6fcff0…`) | 1,654 ms | 115,978 ms (~70×) | 132,443 ms (~80×) |
| `audio.detect-speech-segments@1.1.0` (Silero VAD, 182 chunks) | 670–799 ms | 38,745 ms (~50×) | — |
| `audio.transcribe-speech@1.2.0` (whisper-tiny, 5.35 s clip) | 119,257 ms | killed at 1,500 s | not attempted |
| `core.consent-purpose-check@1.0.0` | 17 ms | 180 ms | 157 ms |

### Why nested stays slow

Fuel-metered wasmi interpreting a capability that itself does dense numeric
work compounds: every guest instruction is interpreted inside an outer wasm
module that is itself interpreted or JIT'd as the outer host. Short
non-model capabilities remain acceptable (consent-check ~10×).

## Direct WebAssembly path (Spec 138)

`ExactModelBrowserHost` loads a digest-verified Spec 138 package and runs
the guest with the platform `WebAssembly` engine. Governance (pins,
signatures, Trace evidence, byte-identical outputs) is unchanged from
native `ExactModelHostConnector`.

### Checked-in lightweight timings

Run from the repo root (no Hugging Face downloads):

```bash
node scripts/perf/direct-wasm-model-bench.mjs
```

This times `fixtures/models/fixture-echo-1.0.0` and
`fixtures/models/digits-onnx-1.0.0` on Node via `ExactModelBrowserHost`.
Expect sub-second warm executes on a developer laptop; print the
JSON summary the script emits and keep it with local evidence if needed.

CI does **not** gate on absolute milliseconds (machine variance). The
harness asserts a successful execute and byte-stable digest checks only
when `TRAVERSE_PERF_ASSERT=1`.

### Full HF bench procedure (gated; not CI)

Heavy models are **not** vendored. To reproduce DistilBERT-NER / Silero VAD /
Whisper on the **chosen Spec 138 path**:

1. Package (or obtain) Spec 138 exact-ref packages for the same weights and
   digests used by the nested prototypes; verify with
   `traverse-cli model verify`.
2. Set `TRAVERSE_PERF_FULL=1` and point the harness (or a local script) at
   those package directories via `TRAVERSE_PERF_MODEL_DIRS` (comma-separated
   paths containing `model.manifest.json` / `model.wasm` / `model.sig.json`).
3. Run native (`ExactModelHostConnector` / embedder) and Node
   `ExactModelBrowserHost` on identical inputs; confirm byte-identical
   outputs and record wall-clock.
4. **Documented bound for `audio.transcribe-speech@1.2.0` on the Spec 138
   path:** complete a 5.35 s whisper-tiny clip on Node within **≤ 3×** the
   native wall-clock for the same package and input (prototype native was
   ~119 s; expect Node direct-wasm in the same order of magnitude, not the
   nested 25+ minute timeout). Re-measure after each runtime/package bump
   and attach numbers to `#1657` / the release notes.

Chrome: run the same packages through a page (or Playwright) that
constructs `ExactModelBrowserHost`; do not use nested `BundleEmbedder.submit`
for these models.

## Async submit

`BundleEmbedder.submitAsync` / `EmbedderTestDouble.submitAsync` accept
`AbortSignal`. Cancel returns `{ status: "cancelled", error: { code:
"cancelled", … } }` and records Trace API `outcome` / phase `cancelled`.
Sync `submit` remains for small nested capabilities. For mid-flight cancel
of a single long nested invoke, run `submitAsync` inside a Worker (README).

## Related

- ADR-0083, Decision 117
- Spec 138, Spec 1402
- Issue `#1657`

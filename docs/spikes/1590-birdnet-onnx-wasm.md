# Spike #1590: BirdNET v2.4 int8 on an import-free wasm32 ONNX guest

- **Date**: 2026-09-30
- **Decision**: 105 (bar), 106 (verdict: **GO**, with conditions)
- **Unblocks**: `#1591` (together with `#1588`)
- **Code**: `spikes/birdnet-onnx/`. The host probe and harness are in `src/`,
  the ABI v2 prototype guest in `guest/`. The build reads the model from
  `$BIRDNET_ONNX`; **no BirdNET weights are committed**.

## What was tested

- **Model:** BirdNET v2.4 global 6k int8 (ONNX, 46.9 MB). Input
  `[batch, 144000]` f32 (3 s at 48 kHz); output 6,522 logits. Licence
  CC BY-NC-SA 4.0: it was read from a local Callweave checkout only.
- **Engine:** tract 0.21 (pure Rust), compiled to `wasm32-unknown-unknown`
  with the ONNX bytes embedded as a data segment.
  - Guest ABI v2 prototype: `model_alloc`, `model_execute`, and
    `model_prepare` for a one-time warm-up.
  - `getrandom` uses a `custom` stub that always errors. Only tract's
    `Random*` ops use it, and BirdNET has none. This keeps the guest
    import-free.
- **Reference:** onnxruntime-web (WASM backend) under Node.
- **Clips:** 20 × 3 s windows (48 kHz mono f32) from 10 local field
  recordings. Those recordings are not redistributable, so only aggregate
  results are reported here.
- **Machine:** Apple M4, 16 GiB, macOS 26.5.2. `wasmi` 2.0 with fuel
  metering on, as in the Swift host; wasmtime 44 for comparison.

## Results

| Build | Engine | Mean / 3 s clip | Worst | Fuel / clip | Peak guest memory |
|---|---|---|---|---|---|
| native (host tract) | — | 0.056 s | — | — | — |
| plain wasm32 | wasmtime | 0.311 s | 0.469 s | — | 178 MiB |
| plain wasm32 | **wasmi** | **3.23 s** | 3.66 s | 28.4×10⁹ | 178 MiB |
| simd128 | wasmtime | 0.152 s | 0.154 s | — | 176 MiB |
| simd128 | **wasmi** | **1.84 s** | 1.88 s | 11.6×10⁹ | 176 MiB |

- **One-time prepare** (parse and optimize): 0.86 s on `wasmi` plain,
  0.72 s with simd; about 0.09 s on wasmtime.
- **Artifact:** the guest `model.wasm` is 60 MB (59.9 MB plain, 60.4 MB
  simd); imports: none.
- **Determinism:** every wasm run is **bit-identical** to every other
  (plain/simd × `wasmi`/wasmtime, 20/20 clips). Conformance vectors hold
  across engines.
- **Against onnxruntime:**
  - top-5 labels match on **20/20** clips, for every build and engine;
  - sigmoid-score differences: median 1.9×10⁻¹⁰, p99.9 3.6×10⁻⁵;
  - worst difference: **2.2×10⁻³** (wasm) and 4.3×10⁻³ (native tract);
  - only 5 of 130,440 outputs exceed 1×10⁻³, all in 2 clips, and all
    mid-range scores around 0.5, where sigmoid amplifies differences
    between int8 kernels. Ranking is never affected.

## Against the Decision 105 bar

| # | Criterion | Result |
|---|---|---|
| 1 | Runs in an import-free wasm32 guest | ✅ |
| 2 | Top-5 labels match the reference; scores within 1×10⁻³ | Labels ✅ 20/20; scores ⚠️ worst 2.2×10⁻³ (int8 kernel rounding) |
| 3 | Peak guest memory < 256 MiB | ✅ 176–178 MiB |
| 4 | < 3 s per 3 s clip on `wasmi`, Apple silicon | ❌ plain (3.23 s); ✅ **simd128 (1.84 s)** |

## Verdict (Decision 106): GO, with conditions

1. **simd128 is required.** The runner ships as a `+simd128` build, and the
   Swift host must enable `wasmi`'s `simd` feature, which means a new
   xcframework.
2. **Int8 cross-engine tolerance:** top-5 identical and
   |Δ sigmoid score| ≤ 5×10⁻³ against onnxruntime. Conformance *between
   Traverse engines* stays byte-identical.
3. **Fuel:** with simd, one inference (11.6×10⁹) fits Swift's default
   `ExactModelHostLimits.maxFuel` (20×10⁹). The plain build (28.4×10⁹)
   would not.

## Caveats

- Latency was measured on an Apple M4 laptop, not a phone. iPhone-class
  CPUs are typically somewhat slower per core, so `#1591` must confirm the
  simd `wasmi` latency on a physical device.
- The 0.7–0.9 s one-time prepare happens when the model host first loads,
  not per clip. Apps should warm up the model before recording.

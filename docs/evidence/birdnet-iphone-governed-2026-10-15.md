# BirdNET on the governed path on iPhone (#1630)

- **Date:** 2026-10-15
- **Issue:** `#1630`, the evidence that closes umbrella `#1623` (Decision 110, rule 13)
- **Governing spec:** `138-governed-exact-model-execution` 0.15.0 (Decisions 106, 110, 112–116)
- **Raw data:** [`birdnet-iphone-governed-2026-10-15.json`](birdnet-iphone-governed-2026-10-15.json)
- **Harness:** [`tools/model-evidence/ios`](../../tools/model-evidence/ios)

## Result

On an **iPhone 14 Pro Max** (`iPhone15,3`, iOS 26.6, build 23G71), with a
**Release** build, BirdNET v2.4 runs on the governed path under the policy
`[coreml, wasm-cpu]` with Core ML's default compute units (`all`):
- the policy selects Core ML;
- p95 is **141.3 ms per 3 s clip**, against the ≤ 1.5 s bar;
- the outputs are within Decision 106's bar against the device's own
  `wasm-cpu` outputs: top-5 identical on 21/21 clips, and a worst |Δ sigmoid|
  of 4.79e-03 (≤ 5×10⁻³).

The same package on `wasm-cpu`, with snapshot reuse, takes p95
**10.0 s** per clip, which misses the bar.

| Run (host Core ML setting) | Placement the policy selected | First call | Steady p50 | Steady p95 | ≤ 1.5 s | vs `wasm-cpu` |
|---|---|---|---|---|---|---|
| no accelerator (`wasm-cpu`, snapshot reuse) | `wasm-cpu` (cpu, metered) | 9.1 s | 9.8 s | 10.0 s | no | reference |
| `all` (**default**) | `coreml` (all, not metered) | 6.9 s | 133.1 ms | **141.3 ms** | **yes** | top-5 21/21, \|Δsig\| ≤ 4.79e-03 |
| `cpu_and_gpu` | `coreml` (cpu_and_gpu) | 2.4 s | 133.2 ms | 133.8 ms | yes | top-5 21/21, \|Δsig\| ≤ 4.79e-03 |
| `cpu_and_neural_engine` | `wasm-cpu`: the Core ML variant **failed its device self-check** | 12.3 s | 10.3 s | 10.5 s | no | (ran on wasm-cpu) |
| `cpu_only` | — | timed out at the 60 s call deadline | — | — | — | — |

**How to read the timings:**
- "First call" includes registration effects: the on-device Core ML compile
  and the 21-case self-check (FR-062), or `model_prepare` on `wasm-cpu`.
- "Steady" is the next 21 calls, timed around `ExactModelHost.execute`. That
  is the whole governed call: policy, rights, placement and evidence.

## What was measured

- **Package:**
  - `birdnet.v2_4-global-6k-int8` 2.4.0, pin digest
    `0d8f031d19978ae64738a5ac2fb3133ce03a323c764936c12afe5ff068936828`;
  - built with `traverse-cli model package-onnx` from BirdNET v2.4 GLOBAL 6K
    int8 ARM ONNX (`tphakala/BirdNET-v2.4@d016ed1`, SHA-256
    `6a2323fe…289521a`), on the audited ONNX runner guest (ABI v3,
    snapshot reuse);
  - signed with the **test-only** key.
- **Core ML variant:** `model convert-coreml` (digest `9b4f1615…`, the same
  bytes as the #1663 spike), added with
  `--f32-dtype 3 --score-transform sigmoid` (schema 2.3.0, Decision 116).
  Its conformance vector is the package's own `wasm-cpu` outputs on the 21
  clips, byte-identical on `wasmtime` and `wasmi`.
- **Clips:** the 21 × 3 s clips (48 kHz mono) from the Decision 113 spike.
- **Host:** `TraverseEmbedder.ExactModelHost` (Swift) on
  `TraverseSwiftHost.xcframework`, built from commit `dbdd444c`
  (traverse-framework/traverse#1705), with `modelUsage: non_commercial` and
  `coreML: CoreMLAcceleratorConfig(computeUnits:)`.
- **Licensing:** the weights (CC BY-NC-SA 4.0) and clips are not committed.
  `tools/model-evidence/ios/README.md` reproduces the run from local copies.

## Findings

1. **The real-time claim holds within this scope.** On an iPhone 14 Pro Max,
   BirdNET runs under the default policy and compute units at p95
   141.3 ms per 3 s clip, about
   71× faster than `wasm-cpu`.
2. **The Neural Engine setting fails the device self-check.** With
   `cpu_and_neural_engine`, the variant does not meet the tolerance on this
   device, so the policy fell back to `wasm-cpu` as FR-062 requires. The
   likely cause is reduced-precision arithmetic on the Neural Engine. The
   default `all` stays the right choice: here it ran within tolerance.
3. **`cpu_only` timed out on its first call.** The compile, the 21-case
   self-check and the fallback all ran inside one 60 s call deadline. That
   is a follow-up.
4. **The governed path adds about 120 ms per call over a raw prediction.**
   The spike measured about 10 ms per raw Core ML prediction on this phone;
   the governed call takes about 134 ms, and macOS shows the same. It is
   well within budget and is a follow-up.

## Per-clip steady timings (ms)

| Clip | Core ML `all` | Core ML `cpu_and_gpu` | `wasm-cpu` |
|---|---|---|---|
| `src1-w001` | 141.3 | 141.4 | 8,353 |
| `src1-w1613` | 141.4 | 133.8 | 8,486 |
| `src1-w807` | 140.5 | 133.7 | 8,549 |
| `src2-w001` | 133.5 | 133.1 | 8,656 |
| `src2-w054` | 133.0 | 132.8 | 8,654 |
| `src2-w107` | 133.1 | 133.2 | 8,693 |
| `src3-w001` | 132.6 | 133.1 | 8,737 |
| `src3-w1226` | 133.1 | 133.6 | 8,953 |
| `src3-w2451` | 132.9 | 133.8 | 9,465 |
| `src4-w001` | 133.1 | 132.8 | 9,625 |
| `src5-w001` | 133.4 | 133.1 | 9,834 |
| `src5-w003` | 132.8 | 133.0 | 9,874 |
| `src5-w005` | 133.1 | 133.4 | 9,991 |
| `src6-w001` | 132.9 | 133.3 | 10,001 |
| `src6-w010` | 133.0 | 133.1 | 10,011 |
| `src6-w019` | 133.2 | 133.7 | 9,916 |
| `src7-w001` | 133.1 | 133.1 | 9,844 |
| `src7-w002` | 133.3 | 133.3 | 9,985 |
| `src8-w001` | 133.4 | 133.0 | 9,975 |
| `src8-w004` | 133.2 | 133.4 | 10,022 |
| `src8-w007` | 133.5 | 133.2 | 10,064 |

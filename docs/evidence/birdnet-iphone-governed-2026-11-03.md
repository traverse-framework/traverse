# BirdNET compute units on iPhone after Decision 120 (#1706, #1708)

- **Date:** 2026-11-03
- **Issues:** `#1706` (Spec 138 scenario 35) and `#1708`
- **Governing spec:** `138-governed-exact-model-execution` 0.16.0 (Decisions 106, 116, 120)
- **Raw data:** [`birdnet-iphone-governed-2026-11-03.json`](birdnet-iphone-governed-2026-11-03.json)
- **Harness:** [`tools/model-evidence/ios`](../../tools/model-evidence/ios), built from commit `9a3cd844`

## Result

On an **iPhone 14 Pro Max** (`iPhone15,3`, iOS 26.6, build 23G71), with a
**Release** build:
- `cpu_only` no longer times out. Under `[coreml, wasm-cpu]` it registers in
  **4.47 s** and its first call succeeds in **4.29 s**, within the 60 s call
  deadline. The call runs on `wasm-cpu`, because the Core ML variant failed
  its device self-check during registration.
- `cpu_and_neural_engine` and `cpu_only` fail the self-check with the same
  numbers: 1 of 21 cases over tolerance, a worst |Δ sigmoid| of
  **5.067×10⁻³** against a limit of 5×10⁻³, and no top-5 mismatches.
- The default `all` still meets the real-time bar: p95 **149.0 ms per 3 s
  clip**, against ≤ 1.5 s.

| Run (host Core ML setting) | Registered placement | Registration | First call | Steady p50 | Steady p95 | ≤ 1.5 s |
|---|---|---|---|---|---|---|
| no accelerator (`wasm-cpu`, snapshot reuse) | `wasm-cpu` | 1.76 s | 3.46 s | 3.67 s | 3.86 s | no |
| `all` (**default**) | `coreml` | 6.94 s | 157.6 ms | 142.9 ms | **149.0 ms** | **yes** |
| `cpu_and_gpu` | `coreml` | 2.99 s | 143.5 ms | 141.8 ms | 147.8 ms | yes |
| `cpu_and_neural_engine` | `wasm-cpu` (self-check failed) | 4.30 s | 4.15 s | 4.18 s | 4.23 s | no |
| `cpu_only` | `wasm-cpu` (self-check failed) | 4.47 s | 4.29 s | 4.21 s | 4.28 s | no |

Registration includes the on-device compile, the 21-case self-check and, on
fallback, `model_prepare`. "Steady" is 21 calls after the first call, timed
around `ExactModelHost.execute`.

## Device self-check per compute unit

| Compute units | Passed | Failed cases | Worst \|Δ sigmoid\| | Worst case | Top-5 mismatches | Attempt |
|---|---|---|---|---|---|---|
| `all` | yes | 0 / 21 | 4.411×10⁻³ | 9 | 0 | 6,411 ms |
| `cpu_and_gpu` | yes | 0 / 21 | 4.411×10⁻³ | 9 | 0 | 2,499 ms |
| `cpu_and_neural_engine` | no | 1 / 21 | 5.067×10⁻³ | 9 | 0 | 2,457 ms |
| `cpu_only` | no | 1 / 21 | 5.067×10⁻³ | 9 | 0 | 2,560 ms |

The tolerance is the variant's declared `max_abs_diff` of 5×10⁻³ and `top_k`
of 5, compared after `score_transform: sigmoid`.

## Findings

1. **Scenario 35 is met.** The package registers, the first call succeeds
   within its deadline, and the registration time is recorded. Decision 120
   moved one-time preparation out of the call, and the first call now takes
   about the same time as the steady calls. The harness did not reproduce
   the earlier 60 s timeout, so its exact cause was not isolated.
2. **This run does not measure Core ML inference on `cpu_only`.** The variant
   fails its self-check there, so every call runs on `wasm-cpu`, as FR-062
   requires.
3. **The miss is not specific to the Neural Engine.** The earlier evidence
   suggested reduced-precision arithmetic on the Neural Engine as the likely
   cause. `cpu_only` cannot use the Neural Engine, and it produces the same
   worst delta on the same case to every recorded digit, so that explanation
   does not hold on its own.
4. **What the numbers suggest, not what they prove.** The settings split into
   two pairs with identical deltas. One reading is that
   `cpu_and_neural_engine` runs this fp32 model on Core ML's CPU path, in line
   with the Decision 113 spike ("fp32 runs on the CPU and GPU"). The harness
   does not record which processor ran each operation, so this is an
   inference.
5. **The miss is narrow.** One clip misses by about 1.3% of the limit, and
   the top-5 is identical on all 21 cases. The fallback is safe and costs
   speed: about 4.2 s per clip instead of about 142 ms. `all` passes with
   about 12% of the limit to spare, so another clip set could give a different
   result.

## What changed from the #1630 run

Do not read the timings as a before and after:
- **Clips:** these are 21 × 3 s clips at 48 kHz, cut from a local recordings
  folder. The original Decision 113 clips are no longer available. The
  self-check numbers are therefore not directly comparable with #1630's
  4.79×10⁻³.
- **Package:** the conformance vector was regenerated from these clips and the
  package was signed again with a **test-only** key, so the pin digest is new:
  `489d685c39b50ad28e37e859a02db6ec5c07e8d303c610163a4bec29e0dbab7d`. The
  model is the same BirdNET v2.4 GLOBAL 6K int8 ARM ONNX (SHA-256
  `6a2323fe…289521a`).
- **`wasm-cpu`:** p95 is 3.86 s here and was 10.0 s in #1630. The cause was not
  investigated. The fallback runs were slower than the reference run, at about
  4.2 s against 3.7 s, and that was not investigated either.

## Limits

- One device, one OS version, one model and one clip set. Each setting ran once.
- The package is signed with a test key, and no host is Certified (Spec 529).
- **Licensing:** the weights (CC BY-NC-SA 4.0) and clips are not committed.
  `tools/model-evidence/ios/README.md` reproduces the run from local copies.

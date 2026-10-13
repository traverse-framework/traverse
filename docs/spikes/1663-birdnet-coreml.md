# Spike #1663: BirdNET v2.4 as a real Core ML ML Program (Decision 112)

- **Date**: 2026-10-13
- **Decision**: 112 (route and bar), 113 (verdict: **GO**)
- **Code**: `spikes/birdnet-coreml/`
  - the pre-pass and converter;
  - the canonical zip;
  - the Mac comparison and the divergence search;
  - the tract int8 reference probe, pinned to the shipped runner's
    `Cargo.lock`;
  - the iOS harness.

  **No BirdNET weights or audio are committed.**

## Inputs

- **Model:** BirdNET v2.4 global 6k int8 ONNX, 46,886,045 bytes, SHA-256
  `6a2323fe8f99c2af4bb325cf8f11b52748a86aa1f2c9e706c1ae7f907289521a`.
  - There is no official BirdNET ONNX; upstream is Keras/TFLite. These bytes
    are identical to `BirdNET_v2.4_int8_arm.onnx` in the
    `tphakala/BirdNET-v2.4` repository on Hugging Face (its published
    `SHA256SUMS`).
  - The spike read the same file from a local Callweave checkout, which is
    the model #1590 and #1591 measured.
  - Licence: CC BY-NC-SA 4.0.
- **Clips:** 21 × 3 s windows (48 kHz mono f32) from 8 local field recordings
  (not redistributable, so only aggregates are reported).
- **Reference:** native tract 0.21.18 with the ONNX runner guest's own
  `Cargo.lock`. #1590 showed every Traverse engine is byte-identical to it.
- **Toolchain:** Python 3.9.6 venv, installed offline from a hash-pinned
  lockfile (`requirements.lock`, SHA-256 `e6a67b7c…`): torch 2.4.0,
  onnx2torch 1.5.15, coremltools 8.1, onnx 1.17.0, numpy 1.26.4.

## Conversion route

`prepass.py` (deterministic, semantics-preserving) → `onnx2torch` →
`torch.jit.trace` → `coremltools` (ML Program, iOS 17, fp32) →
`canonical_zip.py`.

The pre-pass makes three exact rewrites. Each fixes a construct that would
otherwise fail to convert, or convert wrongly:

1. **Explicit padding:** 45 convolutions use `auto_pad=SAME_UPPER`, which
   onnx2torch can't convert. The pass writes the explicit `pads` instead,
   from static shape inference over the fixed 144,000-sample input.
2. **Reverse slices:** 2 full-axis reverse slices (step −1 to `INT_MIN`;
   TensorFlow's `ReverseV2` in the mel front end) become `Gather` with
   reversed indices. **coremltools drops one element when it lowers that
   slice:** 95 rows instead of 96, which corrupted every output (top-5 0/21).
   `find_divergence.py` found it by binary-searching converted subgraphs.
3. **The dynamic int8 matmul:**
   - `DynamicQuantizeLinear` → `MatMulInteger` → `Cast` becomes float ops
     that reproduce DynamicQuantizeLinear exactly (min/max, scale, rounded
     zero point, round and clamp, a centred matmul).
   - The clamps use `Relu` rather than `Min`/`Max`/`Clip`. onnx2torch lowers
     those to `broadcast_to` on 0-d tensors, which coremltools mistypes.
   - The PyTorch model matches the int8 reference within 3×10⁻⁴.

`convert.py` also registers a generic `aten::broadcast_to` converter
(`x + zeros(shape)`).

## Reproducibility

The raw `.mlpackage` is **not** byte-stable. coremltools writes protobuf map
fields in an unstable order, and `Manifest.json` uses random UUIDs; the
weights are identical. `canonical_zip.py` fixes both:
- it re-serializes `model.mlmodel` with protobuf `deterministic=True`;
- it replaces the UUIDs with UUIDv5s of each item's path;
- it zips entries sorted, stored, at 1980-01-01, with mode 0644.

The zip is **byte-identical** across two conversions in one environment and
a third in a freshly installed one. Its digest is always
`9b4f1615fe4807fc2cfbea7c1023bc8841793a313e67e46ab4f1b25af8f38aef`, and the
canonical package predicts identically to the raw one. So the Decision 112
byte-identical bar holds on macOS arm64. Linux CI parity is for slice 1 to
confirm.

## Results

**Accuracy** against the int8 reference, using Decision 106's bar (top-5
identical; |Δ sigmoid| ≤ 5×10⁻³):

| Precision | Mac (M4) result |
|---|---|
| fp32 | **top-5 21/21; worst 8.3×10⁻⁴: passes** |
| fp16 | top-5 0/21; worst > 0.24: fails (the mel front end loses too much in float16) |

fp32 can't run on the Neural Engine, but it doesn't need to.

**Physical iPhone 14 Pro Max** (iOS 26.6, build 23G71), with the canonical
package compiled on the device by `MLModel.compileModel` (0.22 s):

| Compute units | Load | Mean / 3 s clip | p95 | Max | Top-5 | Worst \|Δ sig\| |
|---|---|---|---|---|---|---|
| all | 0.78 s | 80.9 ms | 37.0 ms | 1,246 ms (one outlier) | 21/21 | 8.3×10⁻⁴ |
| cpuAndGPU | 0.69 s | **9.9 ms** | **10.7 ms** | 11.0 ms | 21/21 | 8.3×10⁻⁴ |
| cpuAndNeuralEngine | 0.36 s | 12.8 ms | 13.0 ms | 13.2 ms | 21/21 | 7.7×10⁻⁴ |
| cpuOnly | 0.30 s | 12.8 ms | 13.0 ms | 13.2 ms | 21/21 | 7.7×10⁻⁴ |

For comparison, the governed `wasmi` path on the same phone took 3,900 ms per
clip (#1591). The #1623 bar is ≤ 1,500 ms p95. Every setting is more than
40× under that bar, and accuracy passes everywhere.

The `all` setting had one 1.25 s outlier on the first measured prediction
after warm-up. Its p95 is still 37 ms. Slice 3 should measure whether
`cpuAndGPU` is the better default for this model class.

`iphone-result.json.txt` holds the raw device JSON.

## Implications for slices 1–3

- **Converter:** pin the pre-pass rewrites and the `broadcast_to` converter;
  convert in fp32. Make canonicalisation part of the `mlpackage-zip` writer.
- **CI:** the re-convert check must show the same digest on the pinned Linux
  image. If it can't, it falls back to the tolerance bar (Decision 112
  rule 3).
- **Adapter:** compile on the device (fast) and cache by variant digest. The
  real-time requirement is met with large headroom, so the compute-unit
  default (Decision 112 rule 6) can favour predictability over peak speed.

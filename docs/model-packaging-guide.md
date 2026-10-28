# Packaging a model for Traverse (Spec 138)

This guide is for teams that ship their own model as a Spec 138 exact-ref
package: a signed, digest-pinned WebAssembly guest that runs the same way on
native, browser, and iOS/macOS hosts. Everything here uses `traverse-cli
model` (Decision 105, `#1589`). The host behaviour it targets is described in
[`exact-ref-model-execution.md`](exact-ref-model-execution.md).

## Install the CLI

Package models with the published Traverse CLI. The crate is
**`traverse-cli-rs`** (binary `traverse-cli`). It requires **rustc ≥ 1.94**.

```bash
rustup toolchain install 1.94.0
cargo +1.94.0 install traverse-cli-rs --version =0.15.0 --locked
```

> **Warning:** crates.io [`traverse-cli`](https://crates.io/crates/traverse-cli)
> is an unrelated Solidity project. `cargo install traverse-cli` installs the
> wrong tool with the same binary name. Always install `traverse-cli-rs`.

For ONNX packaging you also need the audited runner guest. Download it from
the `onnx-runner-v*` artifact release (never marked Latest; same pattern as
`swift-host-v*` / `runtime-wasm-v*`):

```bash
VERSION=0.15.0
curl -fsSL -o runner.wasm \
  "https://github.com/traverse-framework/traverse/releases/download/onnx-runner-v${VERSION}/runner.wasm"
echo "bd95db176d9268f04b06b42c4bd9a380e568a9bfb4b6abeaa8c07c3d79763622  runner.wasm" | shasum -a 256 -c -
```

Pinned digest for `onnx-runner-v0.15.0` / `fixtures/onnx/runner.wasm`:
`bd95db176d9268f04b06b42c4bd9a380e568a9bfb4b6abeaa8c07c3d79763622`.

Digits fixture inputs (no git checkout required):

```bash
TAG=v0.15.0
BASE="https://raw.githubusercontent.com/traverse-framework/traverse/${TAG}/fixtures/onnx"
curl -fsSL -o digits-mlp-1.0.0.onnx "${BASE}/digits-mlp-1.0.0.onnx"
curl -fsSL -o digits-onnx.package.json "${BASE}/digits-onnx.package.json"
```

## What a package is

A package is a directory with three files:

| File | Contents |
|---|---|
| `model.wasm` | An import-free `wasm32` guest exporting `memory` and `model_execute` (plus `model_alloc` for ABI v2+, and `model_prepare` for ABI v3). |
| `model.manifest.json` | Identity, `wasm_digest`, ABI, schemas, `rights`, target, and limits (schema `2.0.0`; `2.1.0` with `rights.derivation`; `2.2.0` with `max_prepare_fuel` / accelerator variants). |
| `model.sig.json` | `{alg: "ed25519", key_id, signature}`: a detached signature over the exact manifest bytes. |

The **pin digest** is the SHA-256 of the exact `model.manifest.json` bytes.
That one hash binds identity, rights, and limits, and transitively the WASM
(through `wasm_digest`). Apps pin that digest; hosts verify everything
locally, with no network access.

## Guest ABI

- **Frames.** Input and output are little-endian frames: `u16` frame
  version (`1`), `u8` dtype, `u8` rank, `rank × u32` dims, `u32` payload
  length, then the payload. The frame version is always `1`, whatever the
  manifest `abi_version`. The guest returns the output length, or a negative
  reject code to fail closed (Spec 138 0.14.0):
  - `-2`: the frame doesn't match the package (`invalid_input` /
    `frame_mismatch`);
  - `-3`: inference failed (`execution_failed` / `inference_failed`);
  - `-4`: the output isn't the declared shape (`model_incompatible` /
    `output_mismatch`);
  - `-1`: generic (`execution_failed` / `guest_rejected`).
- **ABI v1** (`abi_version: 1`). The host writes the input at offset 64 and
  reserves the output region right after it. Use this only for tiny guests
  without a heap.
- **ABI v2** (`abi_version: 2`). The guest exports
  `model_alloc(len) -> ptr`, and the host asks it for the input and output
  buffers. Use this for any guest with a real allocator.
- **ABI v3** (`abi_version: 3`, schema `2.2.0`). ABI v2 plus
  `model_prepare` / snapshot reuse and a required `max_prepare_fuel`. The
  ONNX packager emits this ABI.
- **No imports.** A guest that imports anything is rejected
  (`model verify` reports `imports`).
- Unsupported `schema_version` / `abi_version` values fail registration with
  `manifest_invalid` and a message that names the unsupported version.

### Building a frame

Every input and output crosses the boundary as one little-endian frame:

| Bytes | Field | Value |
| --- | --- | --- |
| 2 | frame version (`u16`) | Always `1`. Not the manifest `abi_version`. |
| 1 | dtype (`u8`) | The package's own code. For an ONNX runner package, `tensor.input_dtype` on input and `tensor.output_dtype` on output. |
| 1 | rank (`u8`) | Number of dims. |
| 4 × rank | dims (`u32` each) | Exactly the package's shape, for example `tensor.input_shape`. |
| 4 | payload length (`u32`) | In **bytes**: values × 4 for `f32`. |
| payload length | payload | Little-endian values. The ONNX runner takes finite `f32` only. |

Dtype codes are not a global enum. Each package declares its own, and the
frame must carry exactly that code. The ONNX runner rejects any frame that
differs from its tensor config in any field with `-2` (`invalid_input` /
`frame_mismatch`).

A worked example: a zero-valued frame for a package declaring
`"input_shape": [160000], "input_dtype": 2`, which is 10 s of 16 kHz mono
`f32` audio:

```python
import struct

shape = [160000]
values = [0.0] * 160000
payload = struct.pack(f"<{len(values)}f", *values)
frame = (
    struct.pack("<HBB", 1, 2, len(shape))      # frame version 1, dtype 2, rank
    + struct.pack(f"<{len(shape)}I", *shape)   # dims
    + struct.pack("<I", len(payload))          # payload length in bytes
    + payload
)
assert len(frame) == 4 + 4 * len(shape) + 4 + 4 * 160000   # 640012 bytes
```

Common mistakes and what they return. Every one is `invalid_input` /
`frame_mismatch` on a host with Spec 138 0.14.0, and a generic reject on an
older one:
- writing the manifest `abi_version` (`2` or `3`) as the frame version;
- counting values instead of bytes in the payload length;
- using a dtype code from another package, or the output dtype on input;
- adding a batch dim the package doesn't declare (for example `[1, 160000]`
  for `[160000]`);
- sending raw PCM with no frame header;
- NaN or infinite values.

## Limits and fuel

The manifest declares `max_memory_bytes`, `max_fuel`, `max_input_bytes`,
`max_output_bytes`, and `max_execution_ms`. Hosts also apply their own
ceilings. A package that declares more than a host allows fails registration
with `host_limit_exceeded`. Pass the target host's ceilings to `model verify`
(`--max-package-bytes`, `--max-memory-bytes`, `--max-fuel`) to check early.

**Fuel is engine-relative.** `wasmtime` and `wasmi` count it differently.
Size `max_fuel` so your conformance vector passes on both engines
(`model conformance` checks this). `max_execution_ms` is the portable bound.

**Measure and run with an optimized interpreter.** `wasmi`'s speed depends on
how it was compiled. YAMNet (about 11.4 billion fuel) runs in about 3.5 s
with an optimized `wasmi`, and takes over 10 minutes with an unoptimized one.
So a debug host misses a `max_execution_ms` that a release host meets.
- This repository's dev and test builds already optimize `wasmi` (#1681), so
  `cargo run -p traverse-cli-rs` and the tests run at near-release speed.
- An app that depends on `traverse-runtime` and builds it in its own debug
  profile should add the same override to its workspace `Cargo.toml`:

  ```toml
  [profile.dev.package.wasmi]
  opt-level = 3
  debug-assertions = false
  [profile.dev.package.wasmi_core]
  opt-level = 3
  debug-assertions = false
  [profile.dev.package.wasmi_ir]
  opt-level = 3
  debug-assertions = false
  [profile.dev.package.wasmi_collections]
  opt-level = 3
  debug-assertions = false
  ```

  Keep `debug-assertions = false`. With `wasmi` optimized but debug
  assertions on, a long-running guest overflows the native stack and aborts
  the process instead of failing closed.
- Never set `max_execution_ms` from a debug measurement.

## Rights

`rights` is required, and every field must be non-empty:

| Field | Meaning |
|---|---|
| `license_id` | SPDX identifier of the weights' licence, for example `CC-BY-4.0` or `CC-BY-NC-SA-4.0`. |
| `attribution` | Text an app must be able to show. |
| `redistribution` | A short statement of the redistribution terms. |
| `commercial_use` | `allowed`, `restricted`, or `prohibited`. |
| `source_url` | Where the weights come from. This is informational; identity is always the digest. |

Map licence terms to `commercial_use` as follows:
- **"Non-commercial only"** (for example CC BY-NC): `prohibited`.
- **Commercial use with conditions** (for example field-of-use limits):
  `restricted`. Apps must acknowledge it in their pin.
- **Permissive** (Apache-2.0, MIT, CC BY): `allowed`.
- **Unknown** terms aren't allowed. Settle them before packaging.

Hosts check these rights against the app's declared `model_usage`. For
example, a `commercial` app can't load a `prohibited` package.

`traverse-cli model verify` accepts the same declaration via
`--model-usage commercial|non_commercial`. Without the flag, the CLI still
uses a non-commercial *verification* default (so `prohibited` packages
verify) but reports `rights_record.effective_usage: null` and keeps
`verification_usage` explicit. Pass `--require-model-usage` with no
`--model-usage` to reproduce `usage_undeclared` like Node/Browser/Swift.
Invalid values (for example `research`) fail closed. With `--json`,
argument and usage errors are a JSON object on stdout with
`error.code: "usage_error"` (exit 5).

**Derivatives** (converted, quantized, or fine-tuned from someone else's
weights) use manifest schema `2.1.0` and add `rights.derivation`:

```json
"derivation": {
  "kind": "converted",
  "source_digest": "<sha256 of the source artifact, e.g. the .onnx file>",
  "source_license_id": "CC-BY-NC-SA-4.0",
  "source_commercial_use": "prohibited",
  "source_url": "https://example.org/source-model"
}
```

A package can never be more permissive than its source (`prohibited <
restricted < allowed`). Otherwise `model verify` and every host reject it
with `rights_inconsistent`.

## Trust and signing keys

Trust is **host-owned**. A host trusts a fixed set of Ed25519 public keys,
and an app manifest can never add trust. Your signing key:
- is a 32-byte Ed25519 secret, stored as 64 hex characters in a file
  **outside** any repository;
- is never generated or written by the CLI;
- has a `key_id` of `ed25519:` + hex SHA-256 of its public key.

Hosts that run your package must be configured to trust your public key.
Traverse's own production key, and its custody and rotation, are covered in
`#1567`.

## Worked example

This packages the trained digits MLP from its unsigned manifest and
`model.wasm`. CI runs the same flow
(`crates/traverse-cli/src/model_cli.rs`, the end-to-end test), and the result
is byte-identical to `fixtures/models/digits-mlp-1.0.0`, which the native and
web suites register.

```bash
# 1. Lay out the package (manifest + wasm), then pin digest.
traverse-cli model digest out/digits/model.manifest.json --json

# 2. Sign with a key kept outside the repo.
traverse-cli model sign out/digits/model.manifest.json --key ~/keys/model-signing.hex

# 3. Verify exactly as a host would (exits non-zero on failure).
# Pass --model-usage commercial|non_commercial to match an app's declaration;
# without it, verification_usage stays non_commercial and effective_usage is null.
traverse-cli model verify out/digits --trusted-key <public-key-hex> --model-usage commercial --json

# 4. Generate a cross-engine conformance vector from sample input frames.
traverse-cli model conformance generate out/digits --trusted-key <public-key-hex> \
  --input frames/0.bin --input frames/1.bin --out out/digits/conformance.json

# 5. Re-check it (what reviewers and CI run).
traverse-cli model conformance check out/digits out/digits/conformance.json --trusted-key <public-key-hex>

# 6. Emit the app manifest exact_model_dependencies entry.
traverse-cli model pin out/digits --json
```

The app then declares the pin and its `model_usage`, and the host registers
the three files, for example with `register_package` / `registerPackage`.

## ONNX models: `model package-onnx`

For an ONNX model with fixed named inputs and outputs (one or many), you
don't write a guest at all. `traverse-cli model package-onnx` patches a copy
of the audited, prebuilt runner guest (`fixtures/onnx/runner.wasm`, built
from `crates/traverse-onnx-runner-guest` on tract with `+simd128`):
- it appends the model as one data segment;
- every other section, including the runner's code, is copied byte for byte.

```bash
# With a checkout:
traverse-cli model package-onnx fixtures/onnx/runner.wasm my-model.onnx my-model.package.json out/my-model

# Without a checkout (runner from onnx-runner-v* release; digest must match):
traverse-cli model package-onnx runner.wasm my-model.onnx my-model.package.json out/my-model
```

`package-onnx` fails closed if `<runner.wasm>` is not the audited guest
(sha256 `7a20f87befcb8792b2f321a488145954ca9f1e000442d5984f1c03c8163353b8`).

`my-model.package.json` holds:
- the manifest fields you own: identity, schemas, `rights`, limits;
- the `tensor` config, fixed at package time and validated on every call.
  A mismatched input frame bundle returns `-2`, which the host reports as
  `invalid_input` / `frame_mismatch`. Two shapes are accepted:

  **Legacy single-IO** (still supported; digits-onnx uses this):

  ```json
  "tensor": {
    "input_name": "x",
    "output_name": "y",
    "input_shape": [1, 64],
    "output_shape": [1, 10],
    "input_dtype": 2,
    "output_dtype": 3
  }
  ```

  **Multi-IO** (Hugging Face–style encoders; see
  `fixtures/onnx/hf-text-encoder-toy.package.json`):

  ```json
  "tensor": {
    "inputs": [
      {"name": "input_ids", "shape": [1, 4], "dtype": 6, "element": "i64"},
      {"name": "attention_mask", "shape": [1, 4], "dtype": 6, "element": "i64"}
    ],
    "outputs": [
      {"name": "last_hidden_state", "shape": [1, 4, 8], "dtype": 2, "element": "f32"},
      {"name": "pooler_output", "shape": [1, 8], "dtype": 3, "element": "f32"}
    ]
  }
  ```

  `element` defaults to `f32`. Use `i64` for token ids and masks. Each
  tensor becomes one Spec 138 little-endian frame; the staged input and
  returned output are the **concatenation** of those frames in `inputs` /
  `outputs` order (a frame bundle). `max_input_bytes` / `max_output_bytes`
  must cover the full bundle. Hosts stage and return opaque bytes — web
  `concatGuestFrames` builds a bundle from per-tensor frames;
- a required `source`: `{license_id, commercial_use, url}` of the ONNX
  model.

### Supported ONNX operators

The runner is tract. A graph can use only the operators below. The list is
generated from the runner's pinned `tract-onnx` and checked in CI, so it
always matches what ships.

<!-- onnx-operators:begin (generated; see crates/traverse-onnx-runner-guest/tests/supported_operators.rs) -->

The runner registers these 180 ONNX operators (`tract-onnx` 0.23.9, as pinned in
`crates/traverse-onnx-runner-guest/Cargo.lock`):

`Abs`, `Acos`, `Acosh`, `Add`, `And`, `ArgMax`, `ArgMin`,
`ArrayFeatureExtractor`, `Asin`, `Asinh`, `Atan`, `Atanh`, `AveragePool`,
`BatchNormalization`, `BiasGelu`, `BitShift`, `BitwiseAnd`, `BitwiseNot`,
`BitwiseOr`, `BitwiseXor`, `BlackmanWindow`, `Cast`, `CastLike`,
`CategoryMapper`, `Ceil`, `Celu`, `Clip`, `Compress`, `Concat`, `Constant`,
`ConstantLike`, `ConstantOfShape`, `Conv`, `ConvInteger`, `ConvTranspose`,
`Cos`, `Cosh`, `CumSum`, `DFT`, `DepthToSpace`, `DequantizeLinear`, `Div`,
`Dropout`, `DynamicQuantizeLinear`, `Einsum`, `Elu`, `Equal`, `Erf`, `Exp`,
`Expand`, `EyeLike`, `FastGelu`, `Flatten`, `Floor`, `GRU`, `Gather`,
`GatherBlockQuantized`, `GatherElements`, `GatherND`, `Gelu`, `Gemm`,
`GlobalAveragePool`, `GlobalLpPool`, `GlobalMaxPool`, `Greater`,
`GreaterOrEqual`, `GridSample`, `GroupNormalization`, `HammingWindow`,
`HannWindow`, `HardSigmoid`, `HardSwish`, `Hardmax`, `Identity`, `If`,
`InstanceNormalization`, `IsInf`, `IsNaN`, `LRN`, `LSTM`,
`LayerNormalization`, `LeakyRelu`, `Less`, `LessOrEqual`, `LinearClassifier`,
`LinearRegressor`, `Log`, `LogSoftmax`, `LpNormalization`, `MatMul`,
`MatMulInteger`, `MatMulNBits`, `Max`, `MaxPool`, `Mean`,
`MeanVarianceNormalization`, `MelWeightMatrix`, `Min`, `Mish`, `Mod`, `Mul`,
`Multinomial`, `Neg`, `NonMaxSuppression`, `NonZero`, `Normalizer`, `Not`,
`OneHot`, `Or`, `PRelu`, `Pad`, `ParametricSoftplus`, `Pow`, `QLinearConv`,
`QLinearMatMul`, `QuantizeLinear`, `QuickGelu`, `RMSNormalization`, `RNN`,
`RandomNormal`, `RandomNormalLike`, `RandomUniform`, `RandomUniformLike`,
`Range`, `Reciprocal`, `ReduceL1`, `ReduceL2`, `ReduceLogSum`,
`ReduceLogSumExp`, `ReduceMax`, `ReduceMean`, `ReduceMin`, `ReduceProd`,
`ReduceSum`, `ReduceSumSquare`, `Relu`, `Reshape`, `Resize`,
`RotaryEmbedding`, `Round`, `Rsqrt`, `STFT`, `ScaledTanh`, `Scan`, `Scatter`,
`ScatterElements`, `ScatterND`, `Selu`, `Shape`, `Shrink`, `Sigmoid`, `Sign`,
`SimplifiedLayerNormalization`, `Sin`, `Sinh`, `Size`,
`SkipLayerNormalization`, `SkipSimplifiedLayerNormalization`, `Slice`,
`Softmax`, `Softplus`, `Softsign`, `SpaceToDepth`, `Split`, `Sqrt`,
`Squeeze`, `Sub`, `Sum`, `Swish`, `Tan`, `Tanh`, `ThresholdedRelu`, `Tile`,
`TopK`, `Transpose`, `TreeEnsembleClassifier`, `Trilu`, `Unsqueeze`, `Where`,
`Xor`

<!-- onnx-operators:end -->

A listed operator is necessary but not sufficient:
- tract checks each operator's opset version and attributes when it loads
  the graph;
- shapes must be static after `tensor.symbols` is applied;
- every declared input/output name must match the graph; element types are
  `f32` and/or `i64` as declared in `tensor` (up to 16 inputs and 16
  outputs).

`RandomNormal`, `RandomNormalLike`, `RandomUniform`, `RandomUniformLike` and
`Multinomial` are registered but fail closed. The runner is deterministic
and has no entropy source.

**The preflight below is the authority for any one graph.** If
`package-onnx` succeeds, the graph runs.

**Preflight.** Before it writes anything, the packager runs the package it
just built (Spec 138 FR-072):
- `model_prepare` plus one zero-valued input frame **bundle** built from
  your `tensor` config (zeros in each tensor's declared element type);
- on `wasmtime` and `wasmi`, through the same governed path and audited
  runner every host uses, under your manifest's limits and deadline;
- the outputs must match byte for byte and carry every declared output
  dtype and shape in order.

If it fails, nothing is written. The command exits non-zero and prints
`{"stage":"preflight","engine":...,"error":{"code","reason","message"}}`.
The common causes:

| `code` / `reason` | Likely cause |
| --- | --- |
| `execution_failed` / `inference_failed` | The graph can't run with your `input_shape` (or tract rejects an operator). |
| `model_incompatible` / `output_mismatch` | `output_shape` isn't what the graph produces. |
| `execution_failed`, no reason, "model_prepare ..." | The graph doesn't load: wrong tensor names, or `max_prepare_fuel` / memory too low. |
| `execution_failed`, no reason, "model_execute ..." | `max_fuel` is below what one inference needs. |
| `timeout` | `max_execution_ms` is too short. Run release builds: a debug CLI interprets far more slowly. |

On success the JSON report gains a `preflight` object: the engines, the
input frame size, and the output frame's SHA-256.

The packager writes `model.wasm` and an unsigned schema `2.2.0` manifest
(`abi_version: 3`, with `max_prepare_fuel`) whose `rights.derivation`
records `kind: converted`, the ONNX file's SHA-256, and the `source`
rights. The runner returns raw output values; post-processing such as
sigmoid, labels, and thresholds belongs in app capabilities. Then continue
with `model sign`, `verify`, `conformance`, and `pin` as above.

`--json` is accepted on `package-onnx` (and every other `model` subcommand);
the packaging report is always JSON either way.

**Embedder compatibility.** Schema `2.2.0` / ABI v3 packages need an
embedder newer than `traverse-embedder-web@0.14.0` (tracked by `#1652`).
Published `0.14.0` rejects them with `manifest_invalid`. Use a build from
`main` (or the next release) to register and run them.

For int8 models, cross-engine results inside Traverse stay byte-identical.
Against an external reference runtime, Decision 106 allows top-5 identical
and |Δ sigmoid| ≤ 5×10⁻³.

## Core ML variants: `model convert-coreml` (Spec 138 0.13.3)

An ONNX package (one with `rights.derivation`) can also carry a `coreml`
accelerator variant for Apple hosts. The variant is an `mlpackage-zip`: the
canonical zip of an fp32 Core ML ML Program. The same ONNX and converter give
the same bytes.

The converter is `tools/coreml/convert_variant.py`. Run it in the
environment pinned by `tools/coreml/requirements.lock` (Python 3.9 on macOS
arm64):

```bash
python3.9 -m venv .coreml-venv
.coreml-venv/bin/python -m pip install --require-hashes --no-deps -r tools/coreml/requirements.lock
export TRAVERSE_COREML_PYTHON="$PWD/.coreml-venv/bin/python"

# 1. Convert. Prints source_digest, digest, tool_version (lock-sha256:...) and
#    the pre-pass rewrites it applied.
traverse-cli model convert-coreml model.onnx out/model.mlpackage.zip --json

# 2. Add it to the package, with a conformance vector of wasm-cpu reference
#    outputs. --f32-dtype names each output dtype code holding f32 values.
traverse-cli model add-variant out/pkg --file out/model.mlpackage.zip \
  --conformance vector.json --peak-memory-bytes 33554432 \
  --max-abs-diff 0.005 --top-k 5 \
  --tool traverse-coreml --tool-version lock-sha256:<lock digest> --f32-dtype 3

# 3. Re-sign (add-variant changes the manifest), then verify.
traverse-cli model sign out/pkg/model.manifest.json --key ~/keys/model-signing.hex
```

Run the commands from the repository root, or pass `--converter` with the
script's path (`--python` overrides `TRAVERSE_COREML_PYTHON`). The
converter makes dynamic dimensions static (size 1). Before conversion it
rewrites `SAME_UPPER` padding, reverse slices and dynamic int8 matmuls into
exact equivalents.

**Classifiers that output logits** add `--score-transform sigmoid`. The
device self-check then compares sigmoid scores, `1 / (1 + e^-x)`, under
`--max-abs-diff` and `--top-k` instead of raw logits (Spec 138 0.15.0,
Decision 116). Converted logits often differ by more than 5×10⁻³ while
their scores stay within Decision 106's bar. Without the flag, a variant
that is accurate on scores can fail the raw check, and every call falls
back to `wasm-cpu`. The flag writes manifest schema `2.3.0`, which hosts
newer than 0.15.0 accept. Leave it off for outputs that are already scores
or plain values; those keep schema `2.2.0`.

`fixtures/models/digits-onnx-coreml-1.0.0` is the worked example. CI re-runs
the conversion on a macOS arm64 runner and requires the committed digest
(`scripts/ci/coreml_variant_reconvert_check.sh`). When you change
`requirements.lock`, re-convert and re-add every committed variant.

## Host compatibility

A package runs on any host that supports its manifest schema and guest ABI:

| Host | Engine | ABI v1 / v2 (schemas `2.0.0`, `2.1.0`) | ABI v3 (schema `2.2.0`, what `package-onnx` emits) | Specific reject codes (Spec 138 0.14.0) | Accelerator placements |
| --- | --- | --- | --- | --- | --- |
| Rust (`traverse-runtime`, CLI) | `wasmtime` or `wasmi` | yes | v0.15.0+ | after v0.15.0 | `coreml` on Apple (`traverse-coreml-adapter`) |
| Browser (`traverse-embedder-web`) | browser WebAssembly | yes | v0.15.0+ (npm 0.14.0 rejects it with `manifest_invalid`) | after v0.15.0 | none (`wasm-cpu` only) |
| Swift (`TraverseEmbedder`, xcframework) | `wasmi` | yes | `swift-host-v0.15.0`+ | `swift-host-v0.15.0-1`+ | `coreml` |
| Kotlin / Android (JNI) | `wasmi` | yes | v0.15.0+ | after v0.15.0 | none |
| .NET (P/Invoke) | `wasmi` | yes | v0.15.0+ | after v0.15.0 | none |

Every host gives byte-identical `wasm-cpu` output for the same package and
frame; that's what conformance vectors check. A host from before the
reject codes reports every guest rejection as `resource_exhausted` /
"model returned invalid output length". Read that error on an older host as
"check the frame and the package", not as a resource limit.

## Provenance: two tiers (Spec 138 0.9.0)

- **Traverse-published trained packages** (for example
  `digits-mlp-1.0.0`) carry full reproducible provenance (FR-024 to FR-027):
  - vendored, digest-pinned data;
  - a deterministic in-repo trainer;
  - a byte-identical guest rebuild;
  - an enforced accuracy floor.
- **Third-party packages**, including converted pretrained models, MUST:
  - pass `traverse-cli model verify`;
  - ship a conformance vector that passes `model conformance check`, which
    runs on `wasmi` and `wasmtime`.

  Packages built from a source artifact (for example by the ONNX runner)
  record it in `rights.derivation`.

## Common failures

| `reason` | Fix |
|---|---|
| `signature_invalid` | The manifest bytes changed after signing. Re-run `model sign`. |
| `key_untrusted` | Pass the right `--trusted-key`; hosts must trust your public key. |
| `digest_mismatch` | `model.wasm` doesn't match `wasm_digest`. Update the manifest, then re-sign. |
| `rights_incomplete` | A rights (or derivation) field is empty; `detail.field` names it. |
| `rights_inconsistent` | `commercial_use` is more permissive than `derivation.source_commercial_use`. |
| `manifest_invalid` | Unknown fields, unsupported `schema_version` / `abi_version` (message names the value), bad limits, `derivation` in a `2.0.0` manifest, or WASM imports. |
| `host_limit_exceeded` | The declared limits exceed the host's ceilings. Lower them, or target a larger host. |
| `timeout` only in a debug build | The host's `wasmi` is unoptimized. Use a release build, or add the `wasmi` profile override under "Limits and fuel". |

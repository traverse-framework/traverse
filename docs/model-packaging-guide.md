# Packaging a model for Traverse (Spec 138)

This guide is for teams that ship their own model as a Spec 138 exact-ref
package: a signed, digest-pinned WebAssembly guest that runs the same way on
native, browser, and iOS/macOS hosts. Everything here uses `traverse-cli
model` (Decision 105, `#1589`). The host behaviour it targets is described in
[`exact-ref-model-execution.md`](exact-ref-model-execution.md).

## What a package is

A package is a directory with three files:

| File | Contents |
|---|---|
| `model.wasm` | An import-free `wasm32` guest exporting `memory` and `model_execute` (and `model_alloc` for ABI v2). |
| `model.manifest.json` | Identity, `wasm_digest`, ABI, schemas, `rights`, target, and limits (schema `2.0.0`, or `2.1.0` with `rights.derivation`). |
| `model.sig.json` | `{alg: "ed25519", key_id, signature}`: a detached signature over the exact manifest bytes. |

The **pin digest** is the SHA-256 of the exact `model.manifest.json` bytes.
That one hash binds identity, rights, and limits, and transitively the WASM
(through `wasm_digest`). Apps pin that digest; hosts verify everything
locally, with no network access.

## Guest ABI

- **Frames.** Input and output are little-endian frames: `u16` frame
  version (`1`), `u8` dtype, `u8` rank, `rank × u32` dims, `u32` payload
  length, then the payload. The guest returns the output length, or `-1` to
  fail closed.
- **ABI v1** (`abi_version: 1`). The host writes the input at offset 64 and
  reserves the output region right after it. Use this only for tiny guests
  without a heap.
- **ABI v2** (`abi_version: 2`). The guest exports
  `model_alloc(len) -> ptr`, and the host asks it for the input and output
  buffers. Use this for any guest with a real allocator, such as the ONNX
  runner.
- **No imports.** A guest that imports anything is rejected
  (`model verify` reports `imports`).

## Limits and fuel

The manifest declares `max_memory_bytes`, `max_fuel`, `max_input_bytes`,
`max_output_bytes`, and `max_execution_ms`. Hosts also apply their own
ceilings. A package that declares more than a host allows fails registration
with `host_limit_exceeded`. Pass the target host's ceilings to `model verify`
(`--max-package-bytes`, `--max-memory-bytes`, `--max-fuel`) to check early.

**Fuel is engine-relative.** `wasmtime` and `wasmi` count it differently.
Size `max_fuel` so your conformance vector passes on both engines
(`model conformance` checks this). `max_execution_ms` is the portable bound.

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
traverse-cli model verify out/digits --trusted-key <public-key-hex> --json

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

For an ONNX model with exactly one input tensor and one output tensor, you
don't write a guest at all. `traverse-cli model package-onnx` patches a copy
of the audited, prebuilt runner guest (`fixtures/onnx/runner.wasm`, built
from `crates/traverse-onnx-runner-guest` on tract with `+simd128`):
- it appends the model as one data segment;
- every other section, including the runner's code, is copied byte for byte.

```bash
traverse-cli model package-onnx fixtures/onnx/runner.wasm my-model.onnx my-model.package.json out/my-model
```

`my-model.package.json` holds:
- the manifest fields you own: identity, schemas, `rights`, limits;
- the `tensor` config: input and output names, shapes, and dtypes, fixed at
  package time and validated on every call. A mismatched frame returns `-1`
  and the host fails closed;
- a required `source`: `{license_id, commercial_use, url}` of the ONNX
  model.

The packager writes `model.wasm` and an unsigned schema `2.1.0` manifest
whose `rights.derivation` records `kind: converted`, the ONNX file's
SHA-256, and the `source` rights. The runner returns raw output values;
post-processing such as sigmoid, labels, and thresholds belongs in app
capabilities. Then continue with `model sign`, `verify`, `conformance`, and
`pin` as above.

For int8 models, cross-engine results inside Traverse stay byte-identical.
Against an external reference runtime, Decision 106 allows top-5 identical
and |Δ sigmoid| ≤ 5×10⁻³.

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
| `manifest_invalid` | Unknown fields, a bad schema version, bad limits, `derivation` in a `2.0.0` manifest, or WASM imports. |
| `host_limit_exceeded` | The declared limits exceed the host's ceilings. Lower them, or target a larger host. |

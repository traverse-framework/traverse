# Next Release Notes

Unreleased changes since `v0.15.0` will be recorded here.

## `traverse-mcp sync` and a shared registry-sync crate (`#1751`, Decision 121)

- New `traverse-mcp sync --out <path> [--release index-vN] [--source-repo
  <owner/repo>] [--json]`. It writes the Spec 055 synced-state file that
  `prepare-cache --synced-state` reads, so Mode A no longer needs a
  hand-written state file or `traverse-cli`.
- The write is atomic: on failure no partial file is left and an existing
  file is kept. The output records the `index-vN` tag it used.
- Registry sync moves into the new `traverse-registry-sync` crate, used by
  both `traverse-cli registry sync` (unchanged behavior and output) and
  `traverse-mcp sync`.
- When GitHub reports a `sha256:` digest for the `index.json` asset, sync
  now checks it and fails with `registry_index_digest_mismatch` on a
  mismatch.

## `traverse-mcp` names the invalid metadata binding (`#1752`, Decision 121)

- `prepare-cache` and server startup now run one shared verification-binding
  validator. A synced state that `prepare-cache` accepts is one the server
  starts on. Before, an empty `service_type` or `lifecycle` passed
  `prepare-cache` and then stopped the server.
- The error code stays `registry_metadata_cache_invalid`. The `--json`
  envelope adds `record` (`namespace/id@version`) and `field`, and the
  message names both.
- `prepare-cache` validates before it writes, so a failure keeps any
  existing cache.
- `traverse-embedder` adds `validate_public_metadata` and
  `RegistryCacheError::details`.

## `traverse-mcp` binaries for four targets (`#1753`, Decision 121)

- The `traverse-mcp-vX.Y.Z` release now carries Linux arm64
  (`aarch64-unknown-linux-gnu`) and Windows x64 (`x86_64-pc-windows-msvc`,
  `.zip`) next to Linux x64 and macOS arm64.
- Every target has a SHA-256 checksum and build provenance, and is
  smoke-run (`--version`) before upload. The release notes list each
  asset's SHA-256.
- `workflow_dispatch` takes `dry_run: true` to build, smoke-run, and
  checksum all four targets without attesting or releasing.

## Security: wasmtime 48.0.7 and Rust 1.95 (`#1744`, Decision 119)

- `wasmtime` / `wasmtime-wasi` move from 45.0.3 to 48.0.7 (LTS). This fixes
  critical GHSA-mghf-wjfp-p5xm: on aarch64, Cranelift could compile a large
  `br_table` to a wrong branch target, letting a guest escape the sandbox.
  No API, contract, limit, or trace change.
- **Breaking for source builds:** the toolchain and MSRV move from 1.94 to
  1.95 (wasmtime 48 needs it). Building or `cargo install`ing this release
  needs rustc ≥ 1.95.
- Committed wasm rebuilt on 1.95.0: `runtime.wasm`, the ONNX runner guest
  and the digits-mlp guest. Model fixtures are re-packaged and re-signed;
  conformance outputs are unchanged.
  - `runtime.wasm` sha256 `ec49cb7674b362004b3c395a817f1d764893e1c98d973255b3953ed3c8532226`.
  - Audited ONNX runner sha256
    `ac5591fac5f37435667169e969f9117cc6f81dc66b73cd74d1aef9000f5d3a0a`,
    replacing the `#1735` digest above; `model package-onnx` from this
    source accepts only this runner.

## Security: ONNX runner guest on tract-onnx 0.23.9 (`#1735`)

- The runner guest moves from `tract-onnx` 0.21 to 0.23.9. This drops the
  vulnerable `time` crate (Dependabot alert #20) and `getrandom` 0.2. The
  guest still imports nothing; `getrandom` 0.3 uses the `unsupported` backend.
- New audited runner sha256
  `7a20f87befcb8792b2f321a488145954ca9f1e000442d5984f1c03c8163353b8`
  (Linux CI build, 15.7 MB, was 11.5 MB). `model package-onnx` from this
  source accepts only this runner; the `onnx-runner-v0.15.0` release runner
  pairs with the 0.15.0 CLI until the next `onnx-runner-v*` release.
- Supported ONNX operators grow from 160 to 180 (strict superset).
- `digits-onnx`, `digits-onnx-coreml` and `hf-text-encoder-toy` are
  re-packaged and re-signed. All conformance output frames are byte-identical
  to the 0.21 runner.

## Security: wasmtime / wasmtime-wasi 44 → 45.0.3 (`#1734`)

- Fixes the wasmtime-wasi advisory where WASI hard links bypass `FilePerms`
  for the destination path. Affects the default `wasmtime-executor` feature
  of `traverse-runtime`. No API, contract, limit, or trace change.


## Perf / API: model path routing + async cancellable submit (`#1657`)

- ADR-0083 / Decision 117: model-backed Browser/Node work uses Spec 138
  `ExactModelBrowserHost` (direct `WebAssembly`), not nested wasmi. Nested
  `runtime.wasm` stays for non-model capabilities. Perf report:
  `docs/perf/nested-vs-direct-wasm-1657.md`; harness
  `scripts/perf/direct-wasm-model-bench.mjs` (lightweight fixtures by
  default; HF packages via `TRAVERSE_PERF_FULL=1`).
- `traverse-embedder-web`: `submitAsync` on `BundleEmbedder`,
  `EmbedderTestDouble`, and `TraverseEmbedderApi` with optional
  `AbortSignal`. Cancel returns `status: "cancelled"` and Trace API
  phase/outcome `cancelled`. Sync `submit` unchanged for small work.
- README: async API, Worker guidance, Spec 138 routing for model math.

## Feature: multi-IO ONNX packaging (`#1661`)

- `model package-onnx` packages Hugging Face–style multi-input/multi-output
  ONNX graphs (and keeps legacy single-IO `package.json`). Token-id inputs
  use `element: "i64"`. I/O is a Spec 138 frame bundle.
- Spec 138 → 0.15.1 (FR-073 on top of 0.15.0). Reference package
  `fixtures/models/hf-text-encoder-toy-1.0.0` with signed cross-host
  conformance; digits-onnx packages rebuilt on the new runner.
- Web `concatGuestFrames`, native `concat_guest_frames`, and Swift/Kotlin/.NET
  `GuestFrame` helpers for app-side bundle assembly; hosts still stage opaque bytes.

## Evidence: BirdNET in real time on iPhone on the governed path (`#1630`)

- iPhone 14 Pro Max, Release build, policy `[coreml, wasm-cpu]`, default
  compute units: p95 is **141 ms per 3 s clip** (bar 1.5 s). Top-5 is
  identical and |Δ sigmoid| ≤ 4.8×10⁻³ against `wasm-cpu`. `wasm-cpu`
  alone takes about 10 s per clip. See
  `docs/evidence/birdnet-iphone-governed-2026-10-15.md`.
- New harness `tools/model-evidence/ios` reproduces it. No weights or audio
  are committed.

## Logit outputs compared on their scores: `tolerance.score_transform` (Spec 138 0.15.0, Decision 116, `#1630`)

- A variant's tolerance may declare `score_transform: sigmoid` (default
  `identity`). Declared `f32` outputs are mapped through it before
  `max_abs_diff` / `top_k`, so classifiers that output logits meet
  Decision 106's score bar. Converted BirdNET differs by up to 3.4×10⁻² in
  raw logits but only 4.8×10⁻³ in sigmoid scores.
- New manifest schema `2.3.0`, a superset of `2.2.0`, carries it. A
  non-identity transform in `2.2.0` is `manifest_invalid`.
  `traverse-cli model add-variant --score-transform sigmoid` writes
  `2.3.0`; without the flag it stays on `2.2.0`.
- Native, web, Swift, Kotlin and .NET hosts accept `2.3.0`. Hosts at or
  below 0.15.0 reject it.

## Fix: web embedder accepts variants that declare `f32_dtypes` (`#1703`)

- `traverse-embedder-web@0.15.0` rejected every package whose accelerator
  variant declares `tolerance.f32_dtypes`, including the signed
  `digits-onnx-coreml-1.0.0` fixture, with `manifest_invalid`. Native hosts
  accept the same bytes. The web host now accepts the optional field
  (unique integer dtype codes 0–255), matching native.

## Docs/packaging: CLI install path + ONNX runner distribution (`#1687`)

- README and `docs/model-packaging-guide.md` document `cargo install
  traverse-cli-rs` (rustc ≥ 1.94), and warn that crates.io `traverse-cli` is
  an unrelated Solidity tool.
- Audited ONNX runner guest ships on `onnx-runner-v*` artifact releases with
  sha256 `bd95db176d9268f04b06b42c4bd9a380e568a9bfb4b6abeaa8c07c3d79763622` (matches `fixtures/onnx/runner.wasm`).
- `model package-onnx` fails closed when the given runner digest is not the
  audited guest. CI installs the CLI from a packaged `.crate` and runs the
  digits-onnx package → verify → conformance path.

## Fix: reject invalid web `modelUsage` (`#1685`)

- `ExactModelBrowserHost` and the framed native create path only accept
  `commercial` | `non_commercial`. Any other value fails closed with
  `usage_invalid` and structured `detail` (`field: model_usage`).
- Spec 138 0.14.2; shared rights suite scenario 11.

## Perf: framed host output length without clone (`#1697`)

- `execute` reports `usage.output_bytes` via `ModelIoStore::output_len` (no buffer copy).

## Feature: packaged `traverse-mcp` Mode A binaries (`#1653`)

- Product tags publish `traverse-mcp-vX.Y.Z` with Linux x86_64 + macOS arm64
  archives, `SHA256SUMS`, and build-provenance attestations (never Latest).
- `traverse-mcp --version` reports workspace version and the tagged commit.

## Fix: `model verify --model-usage` and JSON usage errors (`#1686`)

- `traverse-cli model verify` (and `conformance generate|check`) accept
  `--model-usage commercial|non_commercial` and optional
  `--require-model-usage`.
- Without `--model-usage`, verification still uses a non-commercial default
  but the JSON report sets `rights_record.effective_usage` to `null` and
  keeps `verification_usage` explicit (no false app declaration).
- `--model-usage commercial` on a `commercial_use: prohibited` package fails
  with `rights_policy_denied`; invalid values fail closed.
- When `--json` is present, argument/usage errors (malformed keys, unknown
  flags, bad `--model-usage`) emit a JSON error envelope on stdout with
  `error.code: "usage_error"` and a non-zero exit.

## Fix: host-configurable nested limits for `runtime.wasm` (`#1656`)

- Nested capability fuel / memory inside `runtime.wasm` stay defaulted to
  10M / 32 MiB, but hosts can raise them via `traverse_init` `limits` and
  `BundleEmbedder.init({ limits: { fuel, maxMemoryBytes } })`.
- Bundle load / native registration fail closed with
  `resource_requirements_exceed_host` when a capability's declared
  `execution.resources` or WASM initial memory exceeds the host ceiling.
- Spec 1402 FR-012 amended to 1.5.0; contract `execution.resources` is
  optional and backwards-compatible.

## Fix: reproducible `runtime.wasm` build + digest CI (`#1659`)

- `scripts/build-runtime-wasm.sh` rebuilds `runtime.wasm` from
  `traverse-runtime-wasm` via `traverse-native-bridge` with path-stable
  `RUSTFLAGS` and writes `runtime-release.json` from the workspace version.
- `scripts/ci/runtime_wasm_reproducibility_check.sh` fails CI when a rebuild
  digest drifts from `runtime/runtime.wasm.sha256` (or any bundled copy).
- Committed `runtime/runtime-release.json` now reports workspace `0.15.0`
  (was stale `0.12.0`) and a freshly rebuilt digest.

## Breaking: host-side contract input schema enforcement (`#1658`)

- Capability inputs are validated against the contract `inputs.schema` before
  the artifact runs (native runtime path and web `BundleEmbedder` when a
  component declares `contract_path`).
- `additionalProperties: false` is enforced; unknown fields fail with
  `invalid_input` / `RequestInvalid` and JSON Pointer violation paths
  (e.g. `/skip_redaction`).
- `maxLength` on strings is enforced. Raw `WasmExecutor::run_bytes` still
  does not validate — callers must validate or use the governed runtime /
  embedder path.

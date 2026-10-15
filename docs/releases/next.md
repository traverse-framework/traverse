# Next Release Notes

Unreleased changes since `v0.15.0` will be recorded here.

## Docs/packaging: CLI install path + ONNX runner distribution (`#1687`)

- README and `docs/model-packaging-guide.md` document `cargo install
  traverse-cli-rs` (rustc ≥ 1.94), and warn that crates.io `traverse-cli` is
  an unrelated Solidity tool.
- Audited ONNX runner guest ships on `onnx-runner-v*` artifact releases with
  sha256 `f3902703fe79cc161c13b87cf507dcb25cb53ea3c2a68bed6cc753f9df100966` (matches `fixtures/onnx/runner.wasm`).
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

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

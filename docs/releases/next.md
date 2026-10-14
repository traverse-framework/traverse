# Next Release Notes

Unreleased changes since `v0.15.0` will be recorded here.

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

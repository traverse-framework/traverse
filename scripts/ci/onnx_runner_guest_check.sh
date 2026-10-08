#!/usr/bin/env bash
# Decision 105/106, #1591: prove the checked-in ONNX runner guest
# (`fixtures/onnx/runner.wasm`) comes from the reviewed source.
#
# 1. The runner passes its native tests (tract on the committed digits ONNX
#    export, fail-closed tensor checks) and clippy on host and wasm32.
# 2. Rebuilding it with the pinned toolchain, simd128, and registry paths
#    remapped yields byte-identical `fixtures/onnx/runner.wasm` with zero
#    imports, on the canonical builder host (CI's x86_64-unknown-linux-gnu).
#    Cargo hashes `rustc -vV` (which names the host) into every dependency's
#    `-C metadata`, so symbol hashes, and after LTO the code layout, differ
#    per build host: the runner is reproducible per host triple, not across
#    hosts. Other hosts still build it and check imports and exports, but
#    skip the byte comparison. Packages built from it (`traverse-cli model package-onnx`) are
#    checked by the CLI's committed-package reproducibility test.
#
#   bash scripts/ci/onnx_runner_guest_check.sh            # check
#   bash scripts/ci/onnx_runner_guest_check.sh --update   # refresh runner.wasm
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
canonical_host="x86_64-unknown-linux-gnu"
guest_dir="${repo_root}/crates/traverse-onnx-runner-guest"
runner_wasm="${repo_root}/fixtures/onnx/runner.wasm"
target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}/onnx-runner-guest"
cargo_home="${CARGO_HOME:-${HOME}/.cargo}"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

cd "${guest_dir}"
cargo fmt --check
CARGO_TARGET_DIR="${target_dir}" cargo test --locked --quiet
CARGO_TARGET_DIR="${target_dir}" cargo clippy --locked --all-targets -- -D warnings
# RUSTFLAGS replaces .cargo/config.toml's rustflags, so it repeats +simd128.
# Panic locations embed dependency source paths; remap them so the bytes do
# not depend on where CARGO_HOME or the checkout lives.
export RUSTFLAGS="-C target-feature=+simd128 -C link-arg=--export=__stack_pointer --remap-path-prefix=${cargo_home}/registry/src=/cargo/registry/src --remap-path-prefix=${guest_dir}=/traverse-onnx-runner-guest"
CARGO_TARGET_DIR="${target_dir}" cargo clippy --locked --release --target wasm32-unknown-unknown -- -D warnings
CARGO_TARGET_DIR="${target_dir}" cargo build --locked --release --target wasm32-unknown-unknown

built="${target_dir}/wasm32-unknown-unknown/release/traverse_onnx_runner_guest.wasm"
node -e '
const bytes = require("fs").readFileSync(process.argv[1]);
const module = new WebAssembly.Module(bytes);
const imports = WebAssembly.Module.imports(module);
if (imports.length !== 0) {
  console.error(`ONNX runner must import nothing, found ${JSON.stringify(imports)}`);
  process.exit(1);
}
const exports = WebAssembly.Module.exports(module).map((entry) => entry.name);
for (const name of ["memory", "model_alloc", "model_prepare", "model_execute", "TRAVERSE_MODEL_BLOB", "__stack_pointer"]) {
  if (!exports.includes(name)) {
    console.error(`ONNX runner is missing the ${name} export`);
    process.exit(1);
  }
}' "${built}"

if grep -q "${cargo_home}" "${built}"; then
  echo "Rebuilt ONNX runner still embeds ${cargo_home}; path remapping failed." >&2
  exit 1
fi

actual="$(sha256 "${built}")"
host="$(rustc -vV | awk '/^host:/ {print $2}')"
if [[ "${host}" != "${canonical_host}" ]]; then
  if [[ "${1:-}" == "--update" ]]; then
    echo "Refusing --update on ${host}: runner.wasm is built on ${canonical_host} (CI); take the rebuilt artifact from the failing CI job." >&2
    exit 1
  fi
  echo "ONNX runner guest check passed on ${host}: zero imports and required exports (byte comparison runs only on ${canonical_host})."
  exit 0
fi
if [[ "${1:-}" == "--update" ]]; then
  cp "${built}" "${runner_wasm}"
  echo "Updated ${runner_wasm} (${actual}). Re-run traverse-cli model package-onnx for each runner package, then scripts/fixtures/sign-model-fixtures.mjs."
  exit 0
fi
expected="$(sha256 "${runner_wasm}")"
if [[ "${actual}" != "${expected}" ]]; then
  echo "Rebuilt ONNX runner ${actual} does not match checked-in runner.wasm ${expected}." >&2
  echo "Run scripts/ci/onnx_runner_guest_check.sh --update, repackage, and re-sign." >&2
  exit 1
fi

echo "ONNX runner guest check passed: runner.wasm ${actual} rebuilds reproducibly with zero imports."

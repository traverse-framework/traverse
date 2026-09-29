#!/usr/bin/env bash
# Decision 102 / ADR-0077: prove the signed, checked-in trained digits model
# comes from the reviewed source and data.
#
# 1. The vendored dataset and committed weights match their pinned digests.
# 2. The no_std guest passes its host tests (bit-identical to the trainer on
#    the full held-out split) and clippy on both host and wasm32.
# 3. Rebuilding the guest with the pinned toolchain yields byte-identical
#    `fixtures/models/digits-mlp-1.0.0/model.wasm`, which imports nothing.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
guest_dir="${repo_root}/crates/traverse-digits-mlp-guest"
package_wasm="${repo_root}/fixtures/models/digits-mlp-1.0.0/model.wasm"
target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}/digits-mlp-guest"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

(cd "${repo_root}/fixtures/datasets/uci-optdigits" &&
  if command -v sha256sum >/dev/null 2>&1; then sha256sum --check --quiet SHA256SUMS; else shasum -a 256 --check --quiet SHA256SUMS; fi)

weights="${guest_dir}/weights/digits-mlp-1.0.0.bin"
if [[ "$(sha256 "${weights}")" != "$(tr -d '[:space:]' <"${weights}.sha256")" ]]; then
  echo "digits-mlp weights do not match their pinned digest." >&2
  exit 1
fi

cd "${guest_dir}"
cargo fmt --check
CARGO_TARGET_DIR="${target_dir}" cargo test --locked --quiet
CARGO_TARGET_DIR="${target_dir}" cargo clippy --locked --all-targets -- -D warnings
CARGO_TARGET_DIR="${target_dir}" cargo clippy --locked --release --target wasm32-unknown-unknown -- -D warnings
CARGO_TARGET_DIR="${target_dir}" cargo build --locked --release --target wasm32-unknown-unknown

built="${target_dir}/wasm32-unknown-unknown/release/traverse_digits_mlp_guest.wasm"
expected="$(sha256 "${package_wasm}")"
actual="$(sha256 "${built}")"
if [[ "${actual}" != "${expected}" ]]; then
  echo "Rebuilt digits guest ${actual} does not match checked-in model.wasm ${expected}." >&2
  echo "Rebuild with the pinned toolchain, copy it to ${package_wasm}, and re-run scripts/fixtures/sign-model-fixtures.mjs." >&2
  exit 1
fi

node -e '
const bytes = require("fs").readFileSync(process.argv[1]);
const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
if (imports.length !== 0) {
  console.error(`digits guest must import nothing, found ${JSON.stringify(imports)}`);
  process.exit(1);
}' "${built}"

echo "digits-mlp guest check passed: model.wasm ${actual} rebuilds reproducibly with zero imports."

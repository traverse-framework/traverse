#!/usr/bin/env bash
# Build the governed runtime.wasm artifact and companion metadata.
#
# Recipe (Spec 1402 / #1659):
#   - Toolchain: rust-toolchain.toml (currently 1.94.0)
#   - Package: traverse-runtime-wasm via cargo run -p traverse-native-bridge
#   - Target: wasm32-unknown-unknown (release)
#   - Builder sets CARGO_INCREMENTAL=0, SOURCE_DATE_EPOCH, and
#     --remap-path-prefix for path-stable object code
#   - runtime-release.json.runtime_version comes from workspace CARGO_PKG_VERSION
#
# Usage:
#   bash scripts/build-runtime-wasm.sh           # temp dir; prints path + digest
#   bash scripts/build-runtime-wasm.sh --install # updates runtime/ + bundled copies

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

install=0
if [[ "${1:-}" == "--install" ]]; then
  install=1
fi

toolchain_file="${repo_root}/rust-toolchain.toml"
if [[ -f "${toolchain_file}" ]]; then
  channel="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' "${toolchain_file}" | head -1)"
  if [[ -n "${channel}" ]]; then
    host="$(rustc -vV 2>/dev/null | sed -n 's/^host: //p' || true)"
    if [[ -z "${host}" ]]; then
      case "$(uname -s)" in
        Linux) host="$(uname -m)-unknown-linux-gnu" ;;
        *) host="$(uname -m)-apple-darwin" ;;
      esac
    fi
    rustup_bin="${HOME}/.rustup/toolchains/${channel}-${host}/bin"
    if [[ -d "${rustup_bin}" ]]; then
      export PATH="${rustup_bin}:${PATH}"
    fi
    export RUSTUP_TOOLCHAIN="${channel}"
  fi
fi

rustup target add wasm32-unknown-unknown >/dev/null
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/traverse-runtime-wasm.XXXXXX")"
cleanup() {
  rm -rf "${out_dir}"
}
trap cleanup EXIT

echo "build-runtime-wasm: toolchain=$(rustc -V) target=wasm32-unknown-unknown" >&2
cargo run -q -p traverse-native-bridge --bin traverse-native-bridge -- "${out_dir}"

digest="$(python3 - <<PY
import hashlib, pathlib
wasm = pathlib.Path(r"""${out_dir}""") / "runtime.wasm"
print(hashlib.sha256(wasm.read_bytes()).hexdigest())
PY
)"
release_version="$(python3 - <<PY
import json, pathlib
release = pathlib.Path(r"""${out_dir}""") / "runtime-release.json"
print(json.loads(release.read_text())["runtime_version"])
PY
)"

echo "build-runtime-wasm: runtime_version=${release_version} sha256=${digest}" >&2

if [[ "${install}" -eq 1 ]]; then
  install -d "${repo_root}/runtime"
  cp "${out_dir}/runtime.wasm" "${repo_root}/runtime/runtime.wasm"
  cp "${out_dir}/runtime-release.json" "${repo_root}/runtime/runtime-release.json"
  printf 'sha256:%s\n' "${digest}" >"${repo_root}/runtime/runtime.wasm.sha256"

  copies=(
    "examples/applications/audio-analysis/runtime"
    "examples/applications/traverse-starter/runtime"
    "packages/web/TraverseEmbedder/tests/fixtures"
  )
  for rel in "${copies[@]}"; do
    dest="${repo_root}/${rel}"
    mkdir -p "${dest}"
    cp "${repo_root}/runtime/runtime.wasm" "${dest}/runtime.wasm"
    printf 'sha256:%s\n' "${digest}" >"${dest}/runtime.wasm.sha256"
  done
  echo "build-runtime-wasm: installed to runtime/ and ${#copies[@]} bundled copies" >&2
else
  # Print path then digest on stdout for the reproducibility check.
  # Copy out of the temp dir before trap removes it.
  keep="$(mktemp "${TMPDIR:-/tmp}/traverse-runtime-wasm-keep.XXXXXX.wasm")"
  cp "${out_dir}/runtime.wasm" "${keep}"
  echo "${keep}"
  echo "${digest}"
fi

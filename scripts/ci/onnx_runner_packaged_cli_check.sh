#!/usr/bin/env bash
# #1687 DoD: published-style CLI install + digest-pinned runner packaging.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/traverse-1687-target}"
workdir="$(mktemp -d "${TMPDIR:-/tmp}/traverse-1687-packaged-cli.XXXXXX")"
cleanup() { rm -rf "${workdir}"; }
trap cleanup EXIT
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
expected_digest="$(tr -d '[:space:]' < fixtures/onnx/runner.wasm.sha256)"
actual_digest="$(sha256_file fixtures/onnx/runner.wasm)"
if [[ "${actual_digest}" != "${expected_digest}" ]]; then
  echo "digest mismatch ${actual_digest} vs ${expected_digest}" >&2
  exit 1
fi
echo "==> cargo package -p traverse-cli-rs"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" cargo package -p traverse-cli-rs --no-verify --allow-dirty --target-dir "${workdir}/pkg-target"
crate_path="$(find "${workdir}/pkg-target" -name 'traverse-cli-rs-*.crate' | head -n 1)"
test -n "${crate_path}"
mkdir -p "${workdir}/crate"
tar -xzf "${crate_path}" -C "${workdir}/crate"
crate_dir="$(find "${workdir}/crate" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
echo "==> cargo install --path from .crate"
install_root="${workdir}/install"
mkdir -p "${install_root}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" cargo install --path "${crate_dir}" --root "${install_root}" --locked --force
cli="${install_root}/bin/traverse-cli"
test -x "${cli}"
stage="${workdir}/stage"
mkdir -p "${stage}"
cp fixtures/onnx/runner.wasm fixtures/onnx/runner.wasm.sha256 fixtures/onnx/digits-mlp-1.0.0.onnx fixtures/onnx/digits-onnx.package.json "${stage}/"
test "$(sha256_file "${stage}/runner.wasm")" = "${expected_digest}"
out="${workdir}/out/digits-onnx"
mkdir -p "${out}"
echo "==> model package-onnx"
"${cli}" model package-onnx "${stage}/runner.wasm" "${stage}/digits-mlp-1.0.0.onnx" "${stage}/digits-onnx.package.json" "${out}" --json | tee "${workdir}/package-report.json"
report_digest="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["runner_sha256"])' "${workdir}/package-report.json")"
test "${report_digest}" = "${expected_digest}"
key_json=fixtures/models/test-signing-key.json
secret="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["secret_key_hex"])' "${key_json}")"
public="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["public_key_hex"])' "${key_json}")"
printf '%s\n' "${secret}" > "${workdir}/key.hex"
echo "==> sign / verify / conformance"
"${cli}" model sign "${out}/model.manifest.json" --key "${workdir}/key.hex"
"${cli}" model verify "${out}" --trusted-key "${public}" --json | tee "${workdir}/verify.json"
python3 -c 'import json,sys; v=json.load(open(sys.argv[1])); assert v.get("ok") is True, v' "${workdir}/verify.json"
vector=fixtures/models/conformance/signed-digits-onnx.json
python3 -c 'import json,sys; from pathlib import Path; v=json.loads(Path(sys.argv[1]).read_text()); Path(sys.argv[2]).write_bytes(bytes.fromhex(v["cases"][0]["input_frame_hex"]))' "${vector}" "${workdir}/in.bin"
"${cli}" model conformance generate "${out}" --trusted-key "${public}" --input "${workdir}/in.bin" --out "${workdir}/conformance.json"
"${cli}" model conformance check "${out}" "${workdir}/conformance.json" --trusted-key "${public}"
for file in model.wasm model.manifest.json model.sig.json; do
  cmp -s "${out}/${file}" "fixtures/models/digits-onnx-1.0.0/${file}" || { echo "${file} mismatch" >&2; exit 1; }
done
echo "onnx_runner_packaged_cli_check passed"

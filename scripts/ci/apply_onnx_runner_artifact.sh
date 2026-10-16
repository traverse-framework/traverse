#!/usr/bin/env bash
# Apply a Linux-rebuilt ONNX runner guest wasm (CI artifact onnx-runner-rebuilt),
# refresh digest pins, re-package digits-onnx + hf-text-encoder-toy, and re-sign.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

artifact_wasm="${1:-}"
if [[ -z "${artifact_wasm}" || ! -f "${artifact_wasm}" ]]; then
  echo "usage: $0 <path-to-traverse_onnx_runner_guest.wasm>" >&2
  exit 2
fi

export PATH="$(dirname "$(rustup which --toolchain 1.94.0 cargo 2>/dev/null || which cargo)"):${PATH}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/traverse-1661-target}"

cp "${artifact_wasm}" fixtures/onnx/runner.wasm
digest="$(shasum -a 256 fixtures/onnx/runner.wasm | awk '{print $1}')"
printf '%s\n' "${digest}" > fixtures/onnx/runner.wasm.sha256
python3 - <<PIN
from pathlib import Path
import re
import sys
path = Path("crates/traverse-cli/src/model_packaging.rs")
digest = "${digest}"
text = path.read_text()
pattern = r'(pub const AUDITED_ONNX_RUNNER_SHA256: &str =\s*")[0-9a-f]{64}(";)'
replacement = rf'\g<1>{digest}\2'
updated, n = re.subn(pattern, replacement, text, count=1)
if n != 1:
    raise SystemExit("failed to update AUDITED_ONNX_RUNNER_SHA256")
path.write_text(updated)
print(f"AUDITED_ONNX_RUNNER_SHA256={digest}")
PIN

# Patch packaging-guide digest if present.
python3 - <<'DOC'
from pathlib import Path
import re
import sys
path = Path("docs/model-packaging-guide.md")
sha_file = Path("fixtures/onnx/runner.wasm.sha256").read_text().strip()
text = path.read_text()
updated, n = re.subn(r"sha256 `[0-9a-f]{64}`", f"sha256 `{sha_file}`", text, count=1)
if n:
    path.write_text(updated)
DOC

cargo build -p traverse-cli-rs --locked --release
cli="${CARGO_TARGET_DIR}/release/traverse-cli"

rm -rf fixtures/models/digits-onnx-1.0.0
mkdir -p fixtures/models/digits-onnx-1.0.0
"${cli}" model package-onnx \
  fixtures/onnx/runner.wasm \
  fixtures/onnx/digits-mlp-1.0.0.onnx \
  fixtures/onnx/digits-onnx.package.json \
  fixtures/models/digits-onnx-1.0.0

rm -rf fixtures/models/hf-text-encoder-toy-1.0.0
mkdir -p fixtures/models/hf-text-encoder-toy-1.0.0
"${cli}" model package-onnx \
  fixtures/onnx/runner.wasm \
  fixtures/onnx/hf-text-encoder-toy-1.0.0.onnx \
  fixtures/onnx/hf-text-encoder-toy.package.json \
  fixtures/models/hf-text-encoder-toy-1.0.0

node scripts/fixtures/sign-model-fixtures.mjs
echo "Applied runner ${digest}; re-packaged digits-onnx and hf-text-encoder-toy; re-signed fixtures."

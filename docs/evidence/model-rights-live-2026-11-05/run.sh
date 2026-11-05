#!/usr/bin/env bash
# #1786: model-rights handling against the live registry, with published
# packages only. Writes proof/ from an empty .work/ folder. Needs Rust 1.95,
# curl, python3 and network access.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "${here}/../../.." && pwd)"
work="${here}/.work"
mkdir -p "${work}" "${here}/proof/search"
export CARGO_TARGET_DIR="${work}/target"

cargo install traverse-cli-rs --version 0.16.2 --locked --root "${work}/cli"
cli="${work}/cli/bin/traverse-cli"
cd "${work}"

"${cli}" registry sync --workspace rights-live --json > "${here}/proof/sync.json"
tag="$(python3 -I -c 'import json,sys;print(json.load(open(sys.argv[1]))["release_tag"])' "${here}/proof/sync.json")"
curl -sSfL -o "${work}/index.json" "https://github.com/traverse-framework/registry/releases/download/${tag}/index.json"

for id in text.detect-entities text.redact-entities audio.detect-speech-segments audio.transcribe-speech report.translate-fr-semantic; do
  "${cli}" registry inspect "${id}" --workspace rights-live --json > "${here}/proof/inspect-${id}.json"
  "${cli}" registry inspect "${id}" --workspace rights-live > "${here}/proof/inspect-${id}.txt"
done
bash "${here}/search-matrix.sh" "${work}" "${cli}" "${here}/proof/search" > "${here}/proof/search/matrix.txt"

# Exact pins on the five backfilled versions, through the CLI's real resolver.
cp -R "${here}/app" "${work}/app"
"${cli}" app prepare --manifest app/app.manifest.json --workspace rights-live --json > "${here}/proof/prepare.json" || true
"${cli}" app validate --manifest app/app.manifest.json --workspace rights-live --json > "${here}/proof/validate.json" || true
"${cli}" app register --manifest app/app.manifest.json --workspace rights-live --json > "${here}/proof/register.json" || true

# Published traverse-runtime's rights policy and traverse-contracts' parser.
(cd "${here}/harness" && cargo build --release --locked)
"${CARGO_TARGET_DIR}/release/rights-live-harness" \
  "${work}/.traverse/workspaces/rights-live/registry/public/index.json" \
  "${work}/.traverse/cache/sha256" > "${here}/proof/runtime-policy.json"

python3 -I "${here}/compare.py" "${work}" "${repo}"

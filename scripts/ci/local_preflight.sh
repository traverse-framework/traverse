#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "usage: $0 --pr-body <file>" >&2
  exit 2
}

[[ $# -eq 2 && $1 == "--pr-body" ]] || usage
pr_body=$2
[[ -f $pr_body ]] || { echo "local-preflight: PR body file not found: $pr_body" >&2; exit 2; }

target=wasm32-unknown-unknown
if ! rustup target list --installed | grep -qx "$target"; then
  echo "local-preflight: SKIP fixture/build checks: missing Rust target $target" >&2
  echo "local-preflight: remediation: rustup target add $target" >&2
  exit 1
fi

echo "local-preflight: RUN cargo fmt --all --check"
cargo fmt --all --check
echo "local-preflight: RUN repository checks"
bash scripts/ci/repository_checks.sh
echo "local-preflight: RUN Rust checks"
bash scripts/ci/rust_checks.sh
echo "local-preflight: RUN executable WASM smoke checks"
bash scripts/ci/wasm_agent_example_smoke.sh
echo "local-preflight: RUN spec alignment (provide BASE_SHA for changed-file coverage)"
if [[ -n ${BASE_SHA:-} ]]; then
  GITHUB_BASE_SHA="$BASE_SHA" GITHUB_HEAD_SHA="$(git rev-parse HEAD)" \
    bash scripts/ci/spec_alignment_check.sh "$pr_body"
else
  echo "local-preflight: SKIP spec changed-file coverage: set BASE_SHA=<base commit>"
fi
echo "local-preflight: SKIP hosted-only: CodeQL, macOS native certification, cross-platform stress, web embedder package"

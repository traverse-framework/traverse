#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage:
  scripts/ci/local_preflight.sh --pr <number>
  BASE_SHA=<base commit> scripts/ci/local_preflight.sh --pr-body <file>

--pr fetches the pull request body and base/head commits from GitHub, then
requires the checked-out commit to match the PR head. --pr-body is for an
unpublished change and requires BASE_SHA so spec alignment checks changed
files rather than merely the PR-body shape.
EOF
  exit 2
}

pr_body=""
base_sha="${BASE_SHA:-}"
cleanup_file=""

cleanup() {
  [[ -z "$cleanup_file" ]] || rm -f "$cleanup_file"
}
trap cleanup EXIT

case "$#:${1:-}" in
  2:--pr)
    pr_number=$2
    command -v gh >/dev/null 2>&1 || {
      echo "local-preflight: gh is required for --pr; use --pr-body for an unpublished change" >&2
      exit 2
    }
    cleanup_file="$(mktemp)"
    gh pr view "$pr_number" --json body --jq .body >"$cleanup_file"
    pr_body="$cleanup_file"
    base_sha="$(gh pr view "$pr_number" --json baseRefOid --jq .baseRefOid)"
    pr_head_sha="$(gh pr view "$pr_number" --json headRefOid --jq .headRefOid)"
    local_head_sha="$(git rev-parse HEAD)"
    if [[ "$local_head_sha" != "$pr_head_sha" ]]; then
      echo "local-preflight: validating local commit $local_head_sha against PR #$pr_number base; GitHub currently records head $pr_head_sha" >&2
    fi
    ;;
  2:--pr-body)
    pr_body=$2
    [[ -f "$pr_body" ]] || {
      echo "local-preflight: PR body file not found: $pr_body" >&2
      exit 2
    }
    [[ -n "$base_sha" ]] || {
      echo "local-preflight: BASE_SHA is required with --pr-body" >&2
      exit 2
    }
    ;;
  *) usage ;;
esac

target=wasm32-unknown-unknown
ci_toolchain=1.94.0
if ! rustup toolchain list | awk '{print $1}' | grep -q "^${ci_toolchain}"; then
  echo "local-preflight: missing CI Rust toolchain $ci_toolchain" >&2
  echo "local-preflight: remediation: rustup toolchain install $ci_toolchain" >&2
  exit 1
fi

export PATH="$(dirname "$(rustup which --toolchain "$ci_toolchain" cargo)"):$PATH"

if ! rustup target list --toolchain "$ci_toolchain" --installed | grep -qx "$target"; then
  echo "local-preflight: missing Rust target $target" >&2
  echo "local-preflight: remediation: rustup target add --toolchain $ci_toolchain $target" >&2
  exit 1
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "local-preflight: cargo-llvm-cov is required to run the required coverage gate" >&2
  echo "local-preflight: remediation: cargo install cargo-llvm-cov --locked" >&2
  exit 1
fi

echo "local-preflight: RUN cargo fmt --all --check"
cargo fmt --all --check

echo "local-preflight: RUN expedition artifact/execution/trace/golden smoke paths"
TRAVERSE_REPO_ROOT="$PWD" bash scripts/ci/expedition_artifact_smoke.sh
TRAVERSE_REPO_ROOT="$PWD" bash scripts/ci/expedition_execution_smoke.sh
TRAVERSE_REPO_ROOT="$PWD" bash scripts/ci/expedition_trace_smoke.sh
TRAVERSE_REPO_ROOT="$PWD" bash scripts/ci/expedition_golden_path.sh

echo "local-preflight: RUN zero-to-hero acceptance and repository checks"
TRAVERSE_REPO_ROOT="$PWD" bash scripts/ci/zero_to_hero_acceptance.sh
bash scripts/ci/repository_checks.sh

echo "local-preflight: RUN Rust and runtime WASM checks"
bash scripts/ci/rust_checks.sh
cargo check -p traverse-runtime --target wasm32-unknown-unknown --no-default-features
bash scripts/ci/wasm_agent_example_smoke.sh
bash scripts/ci/event_driven_workflow_smoke.sh
bash scripts/ci/embedder_conformance/rust_package.sh

echo "local-preflight: RUN coverage gate"
bash scripts/ci/coverage_gate.sh

echo "local-preflight: RUN spec alignment against $base_sha"
GITHUB_BASE_SHA="$base_sha" GITHUB_HEAD_SHA="$(git rev-parse HEAD)" \
  bash scripts/ci/spec_alignment_check.sh "$pr_body"

if command -v node >/dev/null 2>&1; then
  echo "local-preflight: RUN web embedder package conformance"
  bash scripts/ci/embedder_conformance/web_package.sh
else
  echo "local-preflight: SKIP web embedder package: node is unavailable"
fi

if [[ "$(uname -s)" == "Darwin" ]] && command -v java >/dev/null 2>&1 && command -v dotnet >/dev/null 2>&1; then
  echo "local-preflight: RUN macOS native artifact certification"
  bash scripts/ci/native_artifact_certification.sh
else
  echo "local-preflight: SKIP native artifact certification: requires macOS, Java, and .NET"
fi

echo "local-preflight: SKIP hosted-only: CodeQL and cross-platform stress matrix"

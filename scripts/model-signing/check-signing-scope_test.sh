#!/usr/bin/env bash
# Fixture tests for scripts/model-signing/check-signing-scope.sh
# (Decision 135, #1891, Spec 138 FR-076/FR-077).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
check="${root}/scripts/model-signing/check-signing-scope.sh"
tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

HEAD=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
MAIN=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
OTHER=cccccccccccccccccccccccccccccccccccccccc
failures=0

pulls() {
  printf '%s\n' "$1" > "${tmpdir}/pulls.json"
}
files() {
  printf '%s\n' "$@" > "${tmpdir}/files.txt"
}
run() {
  SIGN_SHA="${SHA:-${HEAD}}" SIGN_MAIN_SHA="${MAIN}" \
    SIGN_PULLS_JSON="${tmpdir}/pulls.json" SIGN_FILES="${tmpdir}/files.txt" \
    bash "${check}" "$@"
}
expect_ok() {
  local name="$1" want="$2"
  shift 2
  local got
  if got="$(run "$@" 2>"${tmpdir}/err")" && [[ "${got}" == "${want}" ]]; then
    echo "ok - ${name}"
  else
    echo "not ok - ${name}: got '${got:-}' ($(cat "${tmpdir}/err"))" >&2
    failures=$((failures + 1))
  fi
}
expect_refused() {
  local name="$1" reason="$2"
  shift 2
  if run "$@" >/dev/null 2>"${tmpdir}/err"; then
    echo "not ok - ${name}: accepted" >&2
    failures=$((failures + 1))
  elif grep -q -- "${reason}" "${tmpdir}/err"; then
    echo "ok - ${name}"
  else
    echo "not ok - ${name}: wrong reason: $(cat "${tmpdir}/err")" >&2
    failures=$((failures + 1))
  fi
}

open_pr() {
  printf '{"number":%s,"state":"open","base":{"ref":"%s"},"head":{"sha":"%s"}}' "$1" "$2" "$3"
}

pulls "[$(open_pr 42 main "${HEAD}"), $(open_pr 7 main "${OTHER}")]"
files fixtures/models/digits-mlp-1.0.1/model.manifest.json \
  fixtures/models/digits-mlp-1.0.1/model.wasm \
  scripts/fixtures/sign-model-fixtures.mjs
expect_ok "PR head touching only the listed package and the generator" "pr 42" digits-mlp-1.0.1

files fixtures/models/digits-mlp-1.0.1/model.manifest.json
expect_ok "PR head touching only the package" "pr 42" digits-mlp-1.0.1

files fixtures/models/a-1.0.0/model.manifest.json fixtures/models/b-1.0.0/model.manifest.json
expect_ok "several listed packages" "pr 42" a-1.0.0 b-1.0.0

SHA="${MAIN}" expect_ok "head of main (rotation re-sign)" "main" digits-mlp-1.0.0

files fixtures/models/digits-mlp-1.0.1/model.manifest.json scripts/model-signing/sign-packages.sh
expect_refused "a script outside the packages" "outside the listed packages" digits-mlp-1.0.1

files fixtures/models/digits-mlp-1.0.1/model.manifest.json fixtures/models/conformance/signed-digits-mlp.json
expect_refused "the conformance vector" "outside the listed packages" digits-mlp-1.0.1

files fixtures/models/digits-mlp-1.0.0/model.manifest.json
expect_refused "an unlisted package" "outside the listed packages" digits-mlp-1.0.1

files fixtures/models/digits-mlp-1.0.10/model.manifest.json
expect_refused "a package whose name only shares a prefix" "outside the listed packages" digits-mlp-1.0.1

files ""
expect_refused "a PR with no changes" "changes no files" digits-mlp-1.0.1

files fixtures/models/digits-mlp-1.0.1/model.manifest.json
pulls "[$(open_pr 42 main "${HEAD}")]"
SHA="${OTHER}" expect_refused "a SHA that is not a PR head" "not the head of an open pull request" digits-mlp-1.0.1

pulls '[{"number":42,"state":"closed","base":{"ref":"main"},"head":{"sha":"'"${HEAD}"'"}}]'
expect_refused "a closed PR" "not the head of an open pull request" digits-mlp-1.0.1

pulls "[$(open_pr 42 release "${HEAD}")]"
expect_refused "a PR to another branch" "not the head of an open pull request" digits-mlp-1.0.1

pulls "[$(open_pr 42 main "${HEAD}"), $(open_pr 43 main "${HEAD}")]"
expect_refused "two PRs with the same head" "more than one open pull request" digits-mlp-1.0.1

pulls "[$(open_pr 42 main "${HEAD}")]"
SHA="${HEAD:0:12}" expect_refused "a short SHA" "full 40-character" digits-mlp-1.0.1
SHA="$(printf %s "${HEAD}" | tr a-f A-F)" expect_refused "an uppercase SHA" "full 40-character" digits-mlp-1.0.1
SHA="main" expect_refused "a branch name" "full 40-character" digits-mlp-1.0.1
expect_refused "no packages" "no packages listed"
expect_refused "a path-like package" "not a package directory name" ../keys
expect_refused "a dot-dot package" "not a package directory name" ..

if [[ "${failures}" -gt 0 ]]; then
  echo "${failures} signing-scope test(s) failed" >&2
  exit 1
fi
echo "signing-scope tests passed"

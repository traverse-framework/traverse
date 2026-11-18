#!/usr/bin/env bash
# Decision 135 (#1891), Spec 138 FR-076/FR-077: decide whether the
# `Sign model packages` workflow may sign the listed packages at a pinned
# commit. Runs BEFORE the signing secret is used, from the trusted `main`
# checkout, on data the workflow fetched from the GitHub API.
#
#   SIGN_SHA=<40-hex commit>            the commit to sign at
#   SIGN_MAIN_SHA=<40-hex commit>       the current head of main
#   SIGN_PULLS_JSON=<file>              open PRs: [{number, state, base:{ref}, head:{sha}}]
#   SIGN_FILES=<file>                   changed paths of the matching PR, one per line
#   bash scripts/model-signing/check-signing-scope.sh <package-dir>...
#
# Accepted:
#   - SIGN_SHA is the head of main (re-signing reviewed content, e.g. a key
#     rotation); prints "main".
#   - SIGN_SHA is the head of exactly one open PR to main, and every changed
#     path is under fixtures/models/<listed package>/ or is the fixture
#     generator scripts/fixtures/sign-model-fixtures.mjs; prints "pr <number>".
# Anything else exits 1 with the reason on stderr.
set -euo pipefail

fail() {
  echo "signing scope refused: $*" >&2
  exit 1
}

sha="${SIGN_SHA:-}"
main_sha="${SIGN_MAIN_SHA:-}"
generator="scripts/fixtures/sign-model-fixtures.mjs"

[[ "${sha}" =~ ^[0-9a-f]{40}$ ]] || fail "sha must be a full 40-character lowercase commit SHA"
[[ "${main_sha}" =~ ^[0-9a-f]{40}$ ]] || fail "main head SHA is missing or malformed"
[[ "$#" -gt 0 ]] || fail "no packages listed"
for package in "$@"; do
  [[ "${package}" =~ ^[A-Za-z0-9._-]+$ ]] && [[ "${package}" != "." ]] && [[ "${package}" != ".." ]] ||
    fail "not a package directory name: ${package}"
done

if [[ "${sha}" == "${main_sha}" ]]; then
  echo "main"
  exit 0
fi

pulls="${SIGN_PULLS_JSON:-}"
[[ -n "${pulls}" && -f "${pulls}" ]] || fail "open pull request list is missing"
matches="$(jq -r --arg sha "${sha}" \
  '[.[] | select(.state == "open" and .base.ref == "main" and .head.sha == $sha) | .number] | map(tostring) | join(" ")' \
  "${pulls}")"
read -r -a numbers <<< "${matches}"
case "${#numbers[@]}" in
  0) fail "${sha} is not the head of an open pull request to main" ;;
  1) ;;
  *) fail "${sha} is the head of more than one open pull request (${matches})" ;;
esac

files="${SIGN_FILES:-}"
[[ -n "${files}" && -f "${files}" ]] || fail "changed-file list for PR #${numbers[0]} is missing"
changed=0
outside=()
while IFS= read -r path || [[ -n "${path}" ]]; do
  [[ -z "${path}" ]] && continue
  changed=$((changed + 1))
  allowed=0
  if [[ "${path}" == "${generator}" ]]; then
    allowed=1
  else
    for package in "$@"; do
      if [[ "${path}" == "fixtures/models/${package}/"* ]]; then
        allowed=1
        break
      fi
    done
  fi
  [[ "${allowed}" -eq 1 ]] || outside+=("${path}")
done < "${files}"

[[ "${changed}" -gt 0 ]] || fail "PR #${numbers[0]} changes no files"
if [[ "${#outside[@]}" -gt 0 ]]; then
  fail "PR #${numbers[0]} changes paths outside the listed packages and ${generator}: ${outside[*]}"
fi
echo "pr ${numbers[0]}"

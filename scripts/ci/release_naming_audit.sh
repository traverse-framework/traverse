#!/usr/bin/env bash
# Spec 048 FR-019 (#1574): audit the LIVE GitHub Releases against the naming
# convention, including manual edits made in the GitHub UI. Run by
# .github/workflows/release-naming-audit.yml on every release event and weekly.
#
# - A product tag `vX.Y.Z` must be titled exactly `Traverse vX.Y.Z`.
# - Any other tag (swift-host-v*, runtime-wasm-v*, ...) must NOT be titled
#   `Traverse v...`.
# - The Latest release must be the highest product `vX.Y.Z` release.
set -euo pipefail

repo="${GITHUB_REPOSITORY:-traverse-framework/traverse}"
# RELEASE_NAMING_AUDIT_TSV (tag<TAB>title<TAB>isLatest per line) replaces the
# live listing, for testing the audit without touching real releases.
if [[ -n "${RELEASE_NAMING_AUDIT_TSV:-}" ]]; then
  releases="$(cat "${RELEASE_NAMING_AUDIT_TSV}")"
else
  releases="$(gh release list --repo "${repo}" --limit 200 --json tagName,name,isLatest,isDraft \
    --jq '.[] | select(.isDraft | not) | [.tagName, .name, (.isLatest | tostring)] | @tsv')"
fi

failures=0
latest_tag=""
highest_product=""
while IFS=$'\t' read -r tag name latest; do
  [[ -z "${tag}" ]] && continue
  if [[ "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    if [[ "${name}" != "Traverse ${tag}" ]]; then
      echo "release-naming-audit: ${tag} is titled \"${name}\"; expected \"Traverse ${tag}\"." >&2
      failures=$((failures + 1))
    fi
    if [[ -z "${highest_product}" ]] ||
      [[ "$(printf '%s\n%s\n' "${highest_product#v}" "${tag#v}" | sort -V | tail -1)" == "${tag#v}" ]]; then
      highest_product="${tag}"
    fi
  elif [[ "${name}" =~ ^Traverse[[:space:]]+v ]]; then
    echo "release-naming-audit: artifact release ${tag} is titled \"${name}\"; only product tags vX.Y.Z may use \"Traverse v...\"." >&2
    failures=$((failures + 1))
  fi
  [[ "${latest}" == "true" ]] && latest_tag="${tag}"
done <<<"${releases}"

if [[ -n "${highest_product}" && "${latest_tag}" != "${highest_product}" ]]; then
  echo "release-naming-audit: Latest is \"${latest_tag:-none}\"; expected the highest product release ${highest_product}." >&2
  failures=$((failures + 1))
fi

if [[ "${failures}" -ne 0 ]]; then
  echo "Fix titles with: gh release edit <tag> --title \"...\" (product: \"Traverse vX.Y.Z\"; artifacts: e.g. \"TraverseSwiftHost xcframework (workspace vX.Y.Z)\")." >&2
  exit 1
fi
echo "Release naming audit passed: Latest is ${latest_tag}; every product release is \"Traverse vX.Y.Z\"; no artifact release uses the product title."

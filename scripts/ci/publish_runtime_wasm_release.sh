#!/usr/bin/env bash
# Spec 048 FR-020 (#1784): publish the runtime.wasm artifact release for a
# product tag.
#
# Tag `vX.Y.Z` -> release `runtime-wasm-vX.Y.Z`, with runtime.wasm,
# runtime.wasm.sha256 and runtime-release.json read from the product tag, so
# it publishes the tag's bytes from any checkout. Never Latest.
#
# Immutable releases can't replace assets, so a rerun never re-uploads: an
# existing release must already hold these exact bytes, and any difference
# fails. Every run ends by downloading the assets and comparing them.
#
#   bash scripts/ci/publish_runtime_wasm_release.sh v0.16.1 --verify-only  # pin check only
#   bash scripts/ci/publish_runtime_wasm_release.sh v0.16.1                # publish and verify
set -euo pipefail

usage() {
  echo "Usage: bash scripts/ci/publish_runtime_wasm_release.sh vX.Y.Z [--verify-only]" >&2
  exit 1
}

tag="${1:-}"
[[ "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
verify_only=0
case "${2:-}" in
  "") ;;
  --verify-only) verify_only=1 ;;
  *) usage ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
repo="${GITHUB_REPOSITORY:-traverse-framework/traverse}"
version="${tag#v}"
release_tag="runtime-wasm-${tag}"

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/traverse-runtime-wasm-release.XXXXXX")"
trap 'rm -rf "${work_dir}"' EXIT
assets_dir="${work_dir}/assets"
mkdir -p "${assets_dir}"

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

commit="$(git -C "${repo_root}" rev-parse --verify --quiet "refs/tags/${tag}^{commit}" || true)"
if [[ -z "${commit}" ]]; then
  echo "Tag ${tag} does not exist in this checkout." >&2
  exit 1
fi

for asset in runtime.wasm runtime.wasm.sha256 runtime-release.json; do
  if ! git -C "${repo_root}" show "${commit}:runtime/${asset}" >"${assets_dir}/${asset}"; then
    echo "${tag} has no runtime/${asset}." >&2
    exit 1
  fi
done

metadata="$(python3 - "${assets_dir}/runtime-release.json" <<'PY'
import json
import sys

record = json.load(open(sys.argv[1], encoding="utf-8"))
print(record["runtime_version"], record["bridge_version"], record["sha256"])
PY
)"
read -r runtime_version bridge_version recorded_digest <<<"${metadata}"
actual_digest="$(sha256_of "${assets_dir}/runtime.wasm")"
pinned_digest="$(tr -d '[:space:]' <"${assets_dir}/runtime.wasm.sha256")"

if [[ "${runtime_version}" != "${version}" ]]; then
  echo "runtime/runtime-release.json at ${tag} says runtime_version ${runtime_version}; expected ${version}." >&2
  exit 1
fi
if [[ "${recorded_digest}" != "${actual_digest}" ]]; then
  echo "runtime/runtime.wasm at ${tag} has digest ${actual_digest}; runtime-release.json records ${recorded_digest}." >&2
  exit 1
fi
if [[ "${pinned_digest}" != "sha256:${actual_digest}" ]]; then
  echo "runtime/runtime.wasm at ${tag} has digest ${actual_digest}; runtime.wasm.sha256 records ${pinned_digest}." >&2
  exit 1
fi
echo "runtime.wasm at ${tag} matches its pins (${actual_digest})."

if [[ "${verify_only}" -eq 1 ]]; then
  exit 0
fi

title="runtime.wasm ${tag} (bridge ${bridge_version})"
existing="$(gh release view "${release_tag}" --repo "${repo}" --json name --jq .name 2>/dev/null || true)"
if [[ -z "${existing}" ]]; then
  notes="$(cat <<NOTES
Artifact-only release of the \`runtime.wasm\` orchestrator that \`BundleEmbedder\` requires at \`runtime/runtime.wasm\` in an app bundle.

This is **not** the Traverse product release: that is **Traverse ${tag}** on tag \`${tag}\`. These are the bytes pinned in \`runtime/\` at that tag, never marked Latest.

Asset digest (sha256): \`${actual_digest}\`
NOTES
  )"
  gh release create "${release_tag}" \
    "${assets_dir}/runtime.wasm" "${assets_dir}/runtime.wasm.sha256" "${assets_dir}/runtime-release.json" \
    --repo "${repo}" \
    --target "${commit}" \
    --title "${title}" \
    --notes "${notes}" \
    --latest=false
elif [[ "${existing}" != "${title}" ]]; then
  echo "Release ${release_tag} exists with title \"${existing}\"; expected \"${title}\"." >&2
  exit 1
fi

published_dir="${work_dir}/published"
mkdir -p "${published_dir}"
gh release download "${release_tag}" --repo "${repo}" --dir "${published_dir}"
for asset in runtime.wasm runtime.wasm.sha256 runtime-release.json; do
  if ! cmp -s "${assets_dir}/${asset}" "${published_dir}/${asset}"; then
    echo "Release ${release_tag} asset ${asset} is missing or differs from ${tag}; immutable releases can't replace it." >&2
    exit 1
  fi
done
echo "Release \"${title}\" holds the ${tag} bytes."

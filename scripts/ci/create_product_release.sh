#!/usr/bin/env bash
# Spec 048 FR-016/FR-017 (#1574): create the Traverse product GitHub Release.
#
# Naming convention: tag `vX.Y.Z` -> release titled exactly `Traverse vX.Y.Z`,
# notes from `docs/releases/vX.Y.Z.md`, marked Latest. Artifact releases
# (`swift-host-v*`, `runtime-wasm-v*`, `onnx-runner-v*`) never use this title and are never
# Latest (enforced by scripts/ci/release_naming_check.sh).
#
# The release is notes-only, so this repo's immutable-releases policy (which
# only forbids adding assets after publish) never blocks it. Idempotent: an
# existing release with the exact title is success; any other title fails.
#
#   bash scripts/ci/create_product_release.sh v0.14.0            # CI, Latest
#   bash scripts/ci/create_product_release.sh v0.13.0 --not-latest  # backfill
set -euo pipefail

usage() {
  echo "Usage: bash scripts/ci/create_product_release.sh vX.Y.Z [--not-latest]" >&2
  exit 1
}

tag="${1:-}"
[[ "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
latest_flag="--latest"
case "${2:-}" in
  "") ;;
  --not-latest) latest_flag="--latest=false" ;;
  *) usage ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
repo="${GITHUB_REPOSITORY:-traverse-framework/traverse}"
title="Traverse ${tag}"
notes_source="${repo_root}/docs/releases/${tag}.md"

if [[ ! -f "${notes_source}" ]]; then
  echo "Missing ${notes_source}: every product release needs its release notes." >&2
  exit 1
fi

existing="$(gh release view "${tag}" --repo "${repo}" --json name --jq .name 2>/dev/null || true)"
if [[ -n "${existing}" ]]; then
  if [[ "${existing}" == "${title}" ]]; then
    echo "Release ${tag} already exists as \"${title}\"; nothing to do."
    exit 0
  fi
  echo "Release ${tag} exists with title \"${existing}\"; expected \"${title}\"." >&2
  exit 1
fi

notes_file="$(mktemp)"
trap 'rm -f "${notes_file}"' EXIT
# Drop the H1 (the release title carries it) and make relative links absolute
# at the tag, since GitHub renders release notes outside the repo tree.
python3 - "${notes_source}" "${tag}" "${repo}" >"${notes_file}" <<'PY'
import posixpath
import re
import sys

source, tag, repo = sys.argv[1:4]
text = open(source, encoding="utf-8").read()
lines = text.split("\n")
if lines and lines[0].startswith("# "):
    text = "\n".join(lines[1:]).lstrip("\n")

def absolutize(match):
    label, target = match.group(1), match.group(2)
    if re.match(r"^(https?:|mailto:|#)", target):
        return match.group(0)
    path = posixpath.normpath(posixpath.join("docs/releases", target))
    return f"[{label}](https://github.com/{repo}/blob/{tag}/{path})"

print(re.sub(r"\[([^\]]*)\]\(([^)\s]+)\)", absolutize, text))
PY

gh release create "${tag}" \
  --repo "${repo}" \
  --verify-tag \
  --title "${title}" \
  --notes-file "${notes_file}" \
  "${latest_flag}"

echo "Created release \"${title}\" (${latest_flag})."

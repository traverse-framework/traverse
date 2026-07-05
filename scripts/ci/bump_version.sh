#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: bash scripts/ci/bump_version.sh <new-semver>" >&2
}

if [[ "$#" -ne 1 ]]; then
  usage
  exit 1
fi

new_version="$1"

if [[ ! "${new_version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Invalid semver '${new_version}'. Expected MAJOR.MINOR.PATCH without a leading v." >&2
  exit 1
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Working tree is dirty; commit or discard changes before bumping the version." >&2
  exit 1
fi

tag_name="v${new_version}"
if git rev-parse --verify --quiet "refs/tags/${tag_name}" >/dev/null; then
  echo "Tag ${tag_name} already exists." >&2
  exit 1
fi

tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}"' EXIT

awk -v new_version="${new_version}" '
  BEGIN {
    in_workspace_package = 0
    replacements = 0
  }
  /^\[workspace\.package\]$/ {
    in_workspace_package = 1
    print
    next
  }
  /^\[/ {
    in_workspace_package = 0
  }
  in_workspace_package && $1 == "version" && $2 == "=" {
    print "version = \"" new_version "\""
    replacements += 1
    next
  }
  {
    print
  }
  END {
    if (replacements != 1) {
      exit 1
    }
  }
' Cargo.toml > "${tmp_file}"

mv "${tmp_file}" Cargo.toml

if git diff --quiet -- Cargo.toml; then
  echo "Cargo.toml already has workspace version ${new_version}; no bump commit created." >&2
  exit 1
fi

changed_files="$(git diff --name-only)"
if [[ "${changed_files}" != "Cargo.toml" ]]; then
  echo "Version bump changed unexpected files:" >&2
  echo "${changed_files}" >&2
  exit 1
fi

git add Cargo.toml
git commit -m "chore: bump version to ${tag_name}"
git tag "${tag_name}"

echo "Created commit and local tag ${tag_name}."
echo "Push explicitly with:"
echo "  git push origin main"
echo "  git push origin ${tag_name}"

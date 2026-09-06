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
    in_workspace_dependencies = 0
    package_replacements = 0
    dependency_replacements = 0
  }
  /^\[workspace\.package\]$/ {
    in_workspace_package = 1
    in_workspace_dependencies = 0
    print
    next
  }
  /^\[workspace\.dependencies\]$/ {
    in_workspace_package = 0
    in_workspace_dependencies = 1
    print
    next
  }
  /^\[/ {
    in_workspace_package = 0
    in_workspace_dependencies = 0
  }
  in_workspace_package && $1 == "version" && $2 == "=" {
    print "version = \"" new_version "\""
    package_replacements += 1
    next
  }
  in_workspace_dependencies && /^traverse-[a-zA-Z0-9_-]+ = \{.*version = "[0-9]+\.[0-9]+\.[0-9]+"/ {
    line = $0
    gsub(/version = "[0-9]+\.[0-9]+\.[0-9]+"/, "version = \"" new_version "\"", line)
    print line
    dependency_replacements += 1
    next
  }
  {
    print
  }
  END {
    if (package_replacements != 1 || dependency_replacements < 1) {
      exit 1
    }
  }
' Cargo.toml > "${tmp_file}"

mv "${tmp_file}" Cargo.toml

if git diff --quiet -- Cargo.toml; then
  echo "Cargo.toml already has workspace version ${new_version}; no bump commit created." >&2
  exit 1
fi

# Keep Cargo.lock in step with Cargo.toml. The lockfile records a
# `version = "…"` line for every workspace path crate (the members under
# `crates/`), and a stale value breaks `--locked` builds and `cargo metadata
# --locked` on the release tag. Rewrite only those entries: a `[[package]]`
# block whose `name` starts with `traverse-` and which has no `source` line is
# a local path crate. Registry dependencies (e.g. `traverse-registry`) carry a
# `source` line and are left untouched.
lock_tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}" "${lock_tmp_file}"' EXIT

awk -v new_version="${new_version}" '
  function flush_block(   i) {
    if (is_path_crate) {
      for (i = 0; i < n; i++) {
        if (buf[i] ~ /^version = "[0-9]+\.[0-9]+\.[0-9]+"$/) {
          buf[i] = "version = \"" new_version "\""
          lock_replacements += 1
        }
      }
    }
    for (i = 0; i < n; i++) {
      print buf[i]
    }
    n = 0
    is_path_crate = 0
    has_source = 0
  }
  BEGIN {
    in_block = 0
    n = 0
    is_path_crate = 0
    has_source = 0
    lock_replacements = 0
  }
  /^\[\[package\]\]$/ {
    if (in_block) {
      flush_block()
    }
    in_block = 1
    buf[n++] = $0
    next
  }
  in_block && /^$/ {
    flush_block()
    in_block = 0
    print
    next
  }
  in_block {
    if ($1 == "name" && $2 == "=" && $3 ~ /^"traverse-/ && !has_source) {
      is_path_crate = 1
    }
    if ($1 == "source" && $2 == "=") {
      has_source = 1
      is_path_crate = 0
    }
    buf[n++] = $0
    next
  }
  {
    print
  }
  END {
    if (in_block) {
      flush_block()
    }
    if (lock_replacements < 1) {
      exit 1
    }
  }
' Cargo.lock > "${lock_tmp_file}"

mv "${lock_tmp_file}" Cargo.lock

changed_files="$(git diff --name-only)"
unexpected_files="$(printf '%s\n' "${changed_files}" | grep -vxE 'Cargo\.toml|Cargo\.lock' || true)"
if [[ -n "${unexpected_files}" ]]; then
  echo "Version bump changed unexpected files:" >&2
  echo "${unexpected_files}" >&2
  exit 1
fi

git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to ${tag_name}"
git tag "${tag_name}"

echo "Created commit and local tag ${tag_name}."
echo "Push explicitly with:"
echo "  git push origin main"
echo "  git push origin ${tag_name}"

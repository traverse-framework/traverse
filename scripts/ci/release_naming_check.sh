#!/usr/bin/env bash
# Spec 048 FR-016..FR-018 (#1574): static guard for the GitHub Release naming
# convention, run by scripts/ci/repository_checks.sh on every PR.
#
# 1. Artifact releases created by workflows (`gh release create` in
#    .github/workflows) MUST pass `--latest=false` and MUST NOT use the
#    product title `Traverse v...`.
# 2. The product release is created only by scripts/ci/create_product_release.sh,
#    wired into ci.yml after the crates.io `publish` job.
# 3. docs/releases/v<workspace version>.md MUST exist, so the product release
#    always has its notes when the version is bumped.
# 4. Spec 048 FR-020 (#1784): the product release depends on the runtime.wasm
#    artifact release, and stale pins fail in version-guard before publishing.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"
failures=0
fail() {
  echo "release-naming: $1" >&2
  failures=$((failures + 1))
}

while IFS= read -r workflow; do
  # Each `gh release create` command, joined across `\` continuations.
  while IFS= read -r command; do
    [[ -z "${command}" ]] && continue
    if [[ "${command}" != *"--latest=false"* ]]; then
      fail "${workflow}: artifact 'gh release create' must pass --latest=false: ${command}"
    fi
    if [[ "${command}" =~ --title[[:space:]]+[\"\']?Traverse[[:space:]]+v ]]; then
      fail "${workflow}: only create_product_release.sh may title a release 'Traverse v...': ${command}"
    fi
  done < <(awk '
    /^[[:space:]]*#/ { next }
    /gh release create/ { collecting = 1; buffer = "" }
    collecting {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      continued = (line ~ /\\$/)
      sub(/\\$/, "", line)
      buffer = buffer " " line
      if (!continued) { print buffer; collecting = 0 }
    }
  ' "${workflow}")
done < <(find .github/workflows -name '*.yml' -o -name '*.yaml' | sort)

if ! grep -q 'bash scripts/ci/create_product_release.sh "${GITHUB_REF_NAME}"' .github/workflows/ci.yml; then
  fail ".github/workflows/ci.yml must create the product release via scripts/ci/create_product_release.sh"
fi
if ! awk '/^  github-release:/{found=1} found && /- publish/{ok=1} found && /^  [a-z-]+:$/ && !/github-release/{if(found) exit} END{exit !ok}' .github/workflows/ci.yml; then
  fail ".github/workflows/ci.yml github-release job must depend on the crates.io publish job"
fi

if ! awk '/^  github-release:/{found=1} found && /- runtime-wasm-release/{ok=1} found && /^  [a-z-]+:$/ && !/github-release/{if(found) exit} END{exit !ok}' .github/workflows/ci.yml; then
  fail ".github/workflows/ci.yml github-release job must depend on the runtime-wasm-release job"
fi
if ! awk '/^  runtime-wasm-release:/{found=1; next} found && /^  [a-z-]+:$/{exit} found && /bash scripts\/ci\/publish_runtime_wasm_release.sh "\$\{GITHUB_REF_NAME\}"$/{ok=1} END{exit !ok}' .github/workflows/ci.yml; then
  fail ".github/workflows/ci.yml runtime-wasm-release job must run scripts/ci/publish_runtime_wasm_release.sh"
fi
if ! awk '/^  version-guard:/{found=1; next} found && /^  [a-z-]+:$/{exit} found && /publish_runtime_wasm_release.sh "\$\{GITHUB_REF_NAME\}" --verify-only/{ok=1} END{exit !ok}' .github/workflows/ci.yml; then
  fail ".github/workflows/ci.yml version-guard job must verify the runtime.wasm pins"
fi
if ! grep -q -- '--latest=false' scripts/ci/publish_runtime_wasm_release.sh; then
  fail "scripts/ci/publish_runtime_wasm_release.sh must pass --latest=false"
fi

workspace_version="$(awk '
  /^\[workspace\.package\]$/ { in_package = 1; next }
  /^\[/ { in_package = 0 }
  in_package && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
' Cargo.toml)"
if [[ ! -f "docs/releases/v${workspace_version}.md" ]]; then
  fail "docs/releases/v${workspace_version}.md is missing for workspace version ${workspace_version}"
fi

if [[ "${failures}" -ne 0 ]]; then
  exit 1
fi
echo "Release naming check passed (product: \"Traverse v${workspace_version}\"; artifact releases never Latest)."

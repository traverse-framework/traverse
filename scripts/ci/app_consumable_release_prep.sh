#!/usr/bin/env bash

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)

required_files=(
  "docs/app-consumable-release-checklist.md"
  "docs/app-consumable-consumer-bundle.md"
  "docs/app-consumable-release-artifact.md"
  "docs/app-consumable-package-release-pointer.md"
  "docs/app-consumable-requirements-traceability.md"
  "docs/app-consumable-acceptance.md"
  "docs/youaskm3-integration-validation.md"
  "quickstart.md"
)

for file in "${required_files[@]}"; do
  test -s "${repo_root}/${file}"
done

grep -q "publication bundle" "${repo_root}/docs/app-consumable-release-artifact.md"
grep -q "release checklist reference" "${repo_root}/docs/app-consumable-release-artifact.md"
grep -q "versioned consumer bundle reference" "${repo_root}/docs/app-consumable-release-artifact.md"
grep -q "supported runnable consumer artifact reference" "${repo_root}/docs/app-consumable-release-artifact.md"
grep -q "docs/app-consumable-package-release-pointer.md" "${repo_root}/docs/app-consumable-release-artifact.md"
grep -q "bash scripts/ci/app_consumable_release_prep.sh" "${repo_root}/docs/app-consumable-release-artifact.md"
grep -q "## Release Blockers" "${repo_root}/docs/app-consumable-release-checklist.md"
grep -q "docs/app-consumable-consumer-bundle.md" "${repo_root}/docs/app-consumable-release-checklist.md"
grep -q "release artifact and publication bundle" "${repo_root}/docs/app-consumable-requirements-traceability.md"

# README and docs must have been updated for this release.
# The update-docs skill in Claude Code applies the product-writing principles
# from .specify/memory/readme-principles.md. Run it before tagging.
test -s "${repo_root}/.specify/memory/readme-principles.md"
test -s "${repo_root}/README.md"
test -s "${repo_root}/quickstart.md"
test -s "${repo_root}/docs/what-can-i-build.md"

echo "Traverse v0.1 app-consumable release preparation bundle is ready."

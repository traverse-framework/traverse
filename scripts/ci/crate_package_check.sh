#!/usr/bin/env bash
# Package and verify-build every crate that the tag-only publish job uploads
# (#1849). `cargo publish` packages only each crate's own directory, so an
# `include_str!` or `include_bytes!` that reaches outside it fails only at
# publish time. This check runs the same packaging on every PR, before a tag
# is pushed.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

# Reuse the publish script's crate list so the two never drift apart.
crates=()
while IFS= read -r crate; do
  crates+=("${crate}")
done < <(sed -n '/^crates=(/,/^)/p' scripts/ci/publish_crates.sh | grep -oE '"[^"]+"' | tr -d '"')
if [[ "${#crates[@]}" -eq 0 ]]; then
  echo "crate_package_check: no crates parsed from scripts/ci/publish_crates.sh" >&2
  exit 1
fi

args=()
for crate in "${crates[@]}"; do
  args+=(-p "${crate}")
done

echo "Packaging and verifying: ${crates[*]}"
cargo package --locked "${args[@]}"

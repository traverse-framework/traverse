#!/usr/bin/env bash
# Bumps the workspace version in Cargo.toml, commits, and tags locally.
# Governed by spec 048-semver-publishing-pipeline.
#
# Usage: bash scripts/ci/bump_version.sh <new-version>
#   new-version: bare semver, no leading 'v' (e.g. 1.0.0)
#
# Does NOT push. Run: git push origin main && git push origin v<version>

set -euo pipefail

CARGO_TOML="Cargo.toml"

# --- Validate argument ---
if [[ $# -ne 1 ]]; then
    echo "error: usage: bash scripts/ci/bump_version.sh <new-version>" >&2
    exit 1
fi

NEW_VERSION="$1"

if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: '$NEW_VERSION' is not a valid semver string (expected MAJOR.MINOR.PATCH)" >&2
    exit 1
fi

# --- Refuse dirty working tree ---
if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "error: working tree has uncommitted changes; commit or stash before bumping" >&2
    exit 1
fi

# --- Read current version ---
CURRENT_VERSION=$(grep -m1 '^version = ' "$CARGO_TOML" | sed 's/version = "\(.*\)"/\1/')

if [[ "$NEW_VERSION" == "$CURRENT_VERSION" ]]; then
    echo "error: new version '$NEW_VERSION' is the same as the current version" >&2
    exit 1
fi

echo "Bumping $CURRENT_VERSION → $NEW_VERSION"

# --- Update Cargo.toml ---
# Only update the [workspace.package] version line (first occurrence)
sed -i.bak "0,/^version = \"$CURRENT_VERSION\"/{s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/}" "$CARGO_TOML"
rm -f "${CARGO_TOML}.bak"

# Verify the change landed
if ! grep -q "^version = \"$NEW_VERSION\"" "$CARGO_TOML"; then
    echo "error: failed to update version in $CARGO_TOML" >&2
    exit 1
fi

# --- Update Cargo.lock ---
cargo generate-lockfile --quiet 2>/dev/null || true

# --- Commit and tag ---
git add "$CARGO_TOML" Cargo.lock
git commit -m "chore: bump version to v${NEW_VERSION}"
git tag "v${NEW_VERSION}"

echo "Done. Version bumped to $NEW_VERSION."
echo "Push with: git push origin main && git push origin v${NEW_VERSION}"

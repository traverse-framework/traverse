#!/usr/bin/env bash
# Publishes all six Traverse crates to crates.io in dependency order.
# Governed by spec 048-semver-publishing-pipeline.
#
# Requires CARGO_REGISTRY_TOKEN to be set in the environment.
# Idempotent: re-running on an already-published version is a no-op.

set -euo pipefail

if [[ -z "${CARGO_REGISTRY_TOKEN:-}" ]]; then
    echo "error: CARGO_REGISTRY_TOKEN is not set" >&2
    exit 1
fi

CRATES=(
    traverse-contracts
    traverse-registry
    traverse-runtime
    traverse-mcp
    traverse-cli
    traverse-expedition-wasm
)

PROPAGATION_DELAY=30

publish_crate() {
    local name="$1"
    echo "--- Publishing $name ---"
    if cargo publish -p "$name" 2>&1; then
        echo "$name published."
        return 0
    fi

    # Capture output to detect "already uploaded" (idempotent case)
    local output
    output=$(cargo publish -p "$name" 2>&1 || true)
    if echo "$output" | grep -qi "already uploaded\|already exists"; then
        echo "$name already published at this version — skipping."
        return 0
    fi

    echo "error: failed to publish $name" >&2
    echo "$output" >&2
    return 1
}

for crate in "${CRATES[@]}"; do
    publish_crate "$crate"
    echo "Waiting ${PROPAGATION_DELAY}s for crates.io index propagation..."
    sleep "$PROPAGATION_DELAY"
done

echo "All crates published successfully."

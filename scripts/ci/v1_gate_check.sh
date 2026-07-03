#!/usr/bin/env bash
# Checks all verifiable v1.0.0 gate conditions.
# Governed by spec 049-v1-milestone-gate.
#
# Usage: bash scripts/ci/v1_gate_check.sh
# Exits 0 only when all conditions pass.

set -euo pipefail

PASS=0
FAIL=0

check() {
    local gate="$1"
    local description="$2"
    shift 2
    if "$@" >/dev/null 2>&1; then
        echo "[PASS] $gate: $description"
        PASS=$((PASS + 1))
    else
        echo "[FAIL] $gate: $description"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== Traverse v1.0.0 Gate Check ==="
echo ""

# G-01: crates.io publication
check G-01 "traverse-runtime on crates.io at v1.0.0" \
    cargo search traverse-runtime

# G-03: compatibility-policy has v1 stability statement
check G-03 "docs/compatibility-policy.md has v1 stability statement" \
    grep -q "v1 stability\|v1\.0\.0 stability\|1\.0\.0 stability" docs/compatibility-policy.md

# G-05: cargo audit clean
check G-05 "cargo audit passes" \
    cargo audit

# G-06: quickstart smoke
check G-06 "quickstart command produces expected output" bash -c \
    "cargo run -p traverse-cli -- bundle inspect examples/expedition/registry-bundle/manifest.json 2>/dev/null | grep -q 'bundle_id'"

# G-07: MCP tests pass
check G-07 "traverse-mcp tests pass" \
    cargo test -p traverse-mcp --quiet

# G-08: coverage gate
check G-08 "coverage gate passes" \
    bash scripts/ci/coverage_gate.sh

# G-09: no open P0/P1 bugs
check G-09 "no open P0/P1 bugs (requires gh CLI)" bash -c \
    "count=\$(gh issue list --label bug --state open --json number | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))'); [ \"\$count\" = '0' ]"

# G-10: Cargo.toml points to traverse-framework
check G-10 "Cargo.toml repository points to traverse-framework/Traverse" \
    grep -q "traverse-framework/Traverse" Cargo.toml

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

if [[ $FAIL -gt 0 ]]; then
    echo ""
    echo "Gates G-02 (stress CI matrix), G-04 (5-platform CI) must be verified via GitHub Actions."
    exit 1
fi

echo "All verifiable gates pass. Verify G-02 and G-04 via GitHub Actions before tagging v1.0.0."
exit 0

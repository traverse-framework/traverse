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
check G-01 "all Traverse crates are on crates.io at v1.0.0" bash -c '
    for crate in \
        traverse-cli-rs \
        traverse-contracts \
        traverse-expedition-wasm \
        traverse-mcp \
        traverse-registry \
        traverse-runtime
    do
        cargo search "$crate" --limit 1 | grep -q "^${crate} = \"1\.0\.0\""
    done
'

# G-02: thread pool stress evidence
check G-02 "ThreadPoolExecutor stress tests pass locally" \
    cargo test -p traverse-runtime --test thread_pool_stress -- --ignored

# G-03: compatibility-policy has v1 stability statement
check G-03 "docs/compatibility-policy.md has v1 stability statement" \
    grep -q "v1 stability\|v1\.0\.0 stability\|1\.0\.0 stability" docs/compatibility-policy.md

# G-04: cross-platform CI matrix evidence
check G-04 "latest CI workflow completed successfully" bash -c '
    gh run list --workflow ci.yml --branch main --limit 1 --json conclusion \
        | python3 -c "import json,sys; runs=json.load(sys.stdin); sys.exit(0 if runs and runs[0].get(\"conclusion\") == \"success\" else 1)"
'

# G-05: cargo audit clean
check G-05 "cargo audit passes" \
    cargo audit

# G-06: quickstart smoke
check G-06 "quickstart command produces documented output" bash -c '
    output="$(cargo run -p traverse-cli-rs -- bundle inspect examples/expedition/registry-bundle/manifest.json 2>/dev/null)"
    grep -q "bundle_id: expedition.planning.seed-bundle" <<<"$output"
    grep -q "capabilities: 6" <<<"$output"
    grep -q "workflows: 1" <<<"$output"
'

# G-07: MCP tests pass
check G-07 "traverse-mcp tests pass" \
    cargo test -p traverse-mcp --quiet

# G-08: coverage gate
check G-08 "coverage gate passes" \
    bash scripts/ci/coverage_gate.sh

# G-09: no open P0/P1 bugs
check G-09 "no open P0/P1 bugs (requires gh CLI)" bash -c '
    gh issue list --label bug --state open --json number,labels \
        | python3 -c "import json,sys; issues=json.load(sys.stdin); high=[i for i in issues if any(l.get(\"name\", \"\").lower() in {\"p0\", \"p1\", \"priority:p0\", \"priority:p1\", \"priority: p0\", \"priority: p1\"} for l in i.get(\"labels\", []))]; sys.exit(0 if not high else 1)"
'

# G-10: canonical repository metadata
check G-10 "Cargo.toml and docs point to traverse-framework/traverse" bash -c '
    grep -q "traverse-framework/traverse" Cargo.toml
    grep -q "traverse-framework/traverse/actions/workflows/ci.yml" README.md
    grep -q "github.com/traverse-framework/traverse/releases" README.md
    ! grep -R -E "enricopiovesan/Traverse|github.com/users/enricopiovesan/projects/1" README.md docs Cargo.toml >/dev/null
'

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

if [[ $FAIL -gt 0 ]]; then
    exit 1
fi

echo "All verifiable gates pass. Confirm required CI matrix legs before tagging v1.0.0."
exit 0

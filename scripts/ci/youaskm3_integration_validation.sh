#!/usr/bin/env bash

set -euo pipefail

required_files=(
  "quickstart.md"
  "docs/mcp-consumption-validation.md"
  "docs/youaskm3-integration-validation.md"
  "scripts/ci/react_demo_live_adapter_smoke.sh"
  "scripts/ci/mcp_consumption_validation.sh"
)

for file in "${required_files[@]}"; do
  test -f "$file"
  test -s "$file"
done

grep -q "youaskm3" docs/youaskm3-integration-validation.md
grep -q "echo "Reference app smoke lives in traverse-framework/reference-apps"" docs/youaskm3-integration-validation.md
grep -q "bash scripts/ci/mcp_consumption_validation.sh" docs/youaskm3-integration-validation.md
grep -q "bash scripts/ci/youaskm3_integration_validation.sh" docs/youaskm3-integration-validation.md
grep -q "quickstart.md" docs/youaskm3-integration-validation.md
grep -q "consumer_name: youaskm3" docs/mcp-consumption-validation.md
grep -q "validated_flow_id: youaskm3_mcp_validation" docs/mcp-consumption-validation.md

echo "Reference app smoke lives in traverse-framework/reference-apps"
bash scripts/ci/mcp_consumption_validation.sh

if [[ -n "${YOUASKM3_REPO_ROOT:-}" ]]; then
  test -d "${YOUASKM3_REPO_ROOT}"
  test -f "${YOUASKM3_REPO_ROOT}/README.md"
fi

echo "youaskm3 integration validation passed."

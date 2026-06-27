#!/usr/bin/env bash

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)

required_files=(
  "docs/wasm-agent-authoring-guide.md"
  "examples/templates/executable-capability-package/manifest.template.json"
  "examples/agents/expedition-intent-agent/manifest.json"
  "examples/agents/team-readiness-agent/manifest.json"
)

for file in "${required_files[@]}"; do
  test -s "${repo_root}/${file}"
done

grep -q "Traverse WASM Agent Authoring Guide" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "package_id" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "capability_ref" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "workflow_refs" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "binary" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "constraints" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "model_dependencies" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "governed runtime dependencies" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "must not hardcode Ollama" "${repo_root}/docs/wasm-agent-authoring-guide.md"
if grep -q "documentation-only" "${repo_root}/docs/wasm-agent-authoring-guide.md"; then
  echo "WASM agent guide must not describe model_dependencies as documentation-only." >&2
  exit 1
fi
grep -q "examples/templates/executable-capability-package/manifest.template.json" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "examples/agents/expedition-intent-agent/manifest.json" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "examples/agents/team-readiness-agent/manifest.json" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "traverse-cli agent inspect" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "traverse-cli agent execute" "${repo_root}/docs/wasm-agent-authoring-guide.md"
grep -q "bash scripts/ci/wasm_agent_authoring_guide_smoke.sh" "${repo_root}/docs/wasm-agent-authoring-guide.md"

echo "Traverse WASM agent authoring guide is ready."

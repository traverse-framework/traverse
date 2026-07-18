#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
agent_dir="$repo_root/examples/agents/team-readiness-agent"
manifest_path="$agent_dir/manifest.json"
request_path="$repo_root/examples/agents/runtime-requests/validate-team-readiness.json"

bash "$agent_dir/build-fixture.sh" >/dev/null

inspect_output="$(cargo run -p traverse-cli-rs -- agent inspect "$manifest_path")"
printf '%s\n' "$inspect_output" | grep -q "package_id: expedition.planning.validate-team-readiness-agent"
printf '%s\n' "$inspect_output" | grep -q "capability_id: expedition.planning.validate-team-readiness"
printf '%s\n' "$inspect_output" | grep -q "workflow_refs: expedition.planning.plan-expedition@1.0.0"

execute_output="$(cargo run -p traverse-cli-rs -- agent execute "$manifest_path" "$request_path")"
printf '%s\n' "$execute_output" | grep -q "package_id: expedition.planning.validate-team-readiness-agent"
printf '%s\n' "$execute_output" | grep -q "capability_id: expedition.planning.validate-team-readiness"
printf '%s\n' "$execute_output" | grep -q "status: completed"
printf '%s\n' "$execute_output" | grep -q "readiness_status: ready"

echo "Second WASM AI agent smoke passed."

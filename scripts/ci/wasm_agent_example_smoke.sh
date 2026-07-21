#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
cd "$repo_root"

agent_manifest="examples/agents/expedition-intent-agent/manifest.json"
agent_request="examples/agents/runtime-requests/interpret-expedition-intent.json"

bash examples/agents/expedition-intent-agent/build-fixture.sh >/tmp/traverse-agent-build.out

inspect_output="$(cargo run -q -p traverse-cli-rs -- agent inspect "$agent_manifest")"
printf '%s\n' "$inspect_output"

grep -q "package_id: expedition.planning.interpret-expedition-intent-agent" <<<"$inspect_output"
grep -q "capability_id: expedition.planning.interpret-expedition-intent" <<<"$inspect_output"
grep -q "binary_digest: fnv1a64:b31e3df407cd8e3c" <<<"$inspect_output"

execute_output="$(cargo run -q -p traverse-cli-rs -- agent execute "$agent_manifest" "$agent_request")"
printf '%s\n' "$execute_output"

grep -q "status: completed" <<<"$execute_output"
grep -q "capability_id: expedition.planning.interpret-expedition-intent" <<<"$execute_output"
grep -q "route_preferences: conservative-alpine-push, same-day-return" <<<"$execute_output"

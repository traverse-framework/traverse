#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
cd "$repo_root"

agent_manifest="examples/hello-world/say-hello-agent/manifest.json"
agent_request="examples/hello-world/runtime-requests/say-hello.json"

bash examples/hello-world/say-hello-agent/build-fixture.sh >/tmp/traverse-hello-world-build.out

inspect_output="$(cargo run -q -p traverse-cli-rs -- agent inspect "$agent_manifest")"
printf '%s\n' "$inspect_output"

grep -q "package_id: hello.world.say-hello-agent" <<<"$inspect_output"
grep -q "capability_id: hello.world.say-hello" <<<"$inspect_output"
grep -q "workflow_refs: hello.world.say-hello@1.0.0" <<<"$inspect_output"

execute_output="$(cargo run -q -p traverse-cli-rs -- agent execute "$agent_manifest" "$agent_request")"
printf '%s\n' "$execute_output"

grep -q "status: completed" <<<"$execute_output"
grep -q "capability_id: hello.world.say-hello" <<<"$execute_output"
grep -q "name: Traverse" <<<"$execute_output"
grep -q "greeting: Hello, Traverse!" <<<"$execute_output"

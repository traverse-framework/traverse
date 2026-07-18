#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
cd "$repo_root"

agent_manifest="examples/traverse-starter/process-agent/manifest.json"
agent_request="examples/traverse-starter/runtime-requests/process.json"
app_manifest="examples/applications/traverse-starter/app.manifest.json"

bash examples/traverse-starter/process-agent/build-fixture.sh >/tmp/traverse-starter-build.out

inspect_output="$(cargo run -q -p traverse-cli-rs -- agent inspect "$agent_manifest")"
printf '%s\n' "$inspect_output"

grep -q "package_id: traverse-starter.process-agent" <<<"$inspect_output"
grep -q "capability_id: traverse-starter.process" <<<"$inspect_output"
grep -q "workflow_refs: traverse-starter.process@1.0.0" <<<"$inspect_output"

execute_output="$(cargo run -q -p traverse-cli-rs -- agent execute "$agent_manifest" "$agent_request")"
printf '%s\n' "$execute_output"

grep -q "status: completed" <<<"$execute_output"
grep -q "capability_id: traverse-starter.process" <<<"$execute_output"
grep -q "title:" <<<"$execute_output"
grep -q "tags:" <<<"$execute_output"
grep -q "noteType:" <<<"$execute_output"
grep -q "suggestedNextAction:" <<<"$execute_output"
grep -q "starter_status: complete" <<<"$execute_output"

validate_output="$(cargo run -q -p traverse-cli-rs -- app validate --manifest "$app_manifest" --json)"
printf '%s\n' "$validate_output"

grep -q '"status": "validated"' <<<"$validate_output"
grep -q '"app_id": "traverse-starter"' <<<"$validate_output"
grep -q '"component_id": "traverse-starter.process-component"' <<<"$validate_output"
grep -q '"capability_id": "traverse-starter.process"' <<<"$validate_output"

register_output="$(cargo run -q -p traverse-cli-rs -- app register --manifest "$app_manifest" --workspace local-default --json)"
printf '%s\n' "$register_output"

grep -Eq '"status": "(registered|already_registered)"' <<<"$register_output"
grep -q '"state_scope": "workspace_persisted"' <<<"$register_output"

rm -f .traverse/server.json
cargo run -q -p traverse-cli-rs -- serve --port 0 --allow-unauthenticated >/tmp/traverse-starter-serve.out 2>&1 &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true' EXIT

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if test -s .traverse/server.json; then
    break
  fi
  sleep 1
done

test -s .traverse/server.json
base_url="$(grep -o '"base_url": "[^"]*"' .traverse/server.json | cut -d '"' -f4)"
test -n "$base_url"

http_output="$(curl -sS -X POST "$base_url/v1/workspaces/local-default/execute" \
  -H 'Content-Type: application/json' \
  --data-binary "@$agent_request")"
printf '%s\n' "$http_output"

grep -q '"status":"succeeded"' <<<"$http_output"
grep -q '"title":"Review Traverse starter app registration"' <<<"$http_output"
grep -q '"tags":\["review","traverse","starter"\]' <<<"$http_output"
grep -q '"noteType":"project"' <<<"$http_output"
grep -q '"suggestedNextAction":"expand"' <<<"$http_output"
grep -q '"status":"complete"' <<<"$http_output"

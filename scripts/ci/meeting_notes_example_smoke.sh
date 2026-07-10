#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
cd "$repo_root"

agent_manifest="examples/meeting-notes/process-agent/manifest.json"
agent_request="examples/meeting-notes/runtime-requests/process.json"
app_manifest="apps/meeting-notes/app.manifest.json"

bash examples/meeting-notes/process-agent/build-fixture.sh >/tmp/meeting-notes-build.out

inspect_output="$(cargo run -q -p traverse-cli -- agent inspect "$agent_manifest")"
printf '%s\n' "$inspect_output"

grep -q "package_id: meeting-notes.process-agent" <<<"$inspect_output"
grep -q "capability_id: meeting-notes.process" <<<"$inspect_output"
grep -q "workflow_refs: meeting-notes.process@1.0.0" <<<"$inspect_output"

execute_output="$(cargo run -q -p traverse-cli -- agent execute "$agent_manifest" "$agent_request")"
printf '%s\n' "$execute_output"

grep -q "status: completed" <<<"$execute_output"
grep -q "capability_id: meeting-notes.process" <<<"$execute_output"
grep -q "summary: Kickoff notes for Traverse reference app." <<<"$execute_output"
grep -q "action_items: 2" <<<"$execute_output"
grep -q "decisions: 1" <<<"$execute_output"
grep -q "follow_ups: 1" <<<"$execute_output"

validate_output="$(cargo run -q -p traverse-cli -- app validate --manifest "$app_manifest" --json)"
printf '%s\n' "$validate_output"

grep -q '"status": "validated"' <<<"$validate_output"
grep -q '"app_id": "meeting-notes"' <<<"$validate_output"
grep -q '"component_id": "meeting-notes.process-component"' <<<"$validate_output"
grep -q '"capability_id": "meeting-notes.process"' <<<"$validate_output"

register_output="$(cargo run -q -p traverse-cli -- app register --manifest "$app_manifest" --workspace local-default --json)"
printf '%s\n' "$register_output"

grep -Eq '"status": "(registered|already_registered)"' <<<"$register_output"
grep -q '"state_scope": "workspace_persisted"' <<<"$register_output"

rm -f .traverse/server.json
cargo run -q -p traverse-cli -- serve --port 0 --allow-unauthenticated >/tmp/meeting-notes-serve.out 2>&1 &
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
grep -q '"summary":"Kickoff notes for Traverse reference app.' <<<"$http_output"
grep -q '"action_items":\[' <<<"$http_output"
grep -q '"decisions":\[' <<<"$http_output"
grep -q '"follow_ups":\[' <<<"$http_output"

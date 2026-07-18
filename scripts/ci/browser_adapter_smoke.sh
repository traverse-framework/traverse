#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
tmpdir="$(mktemp -d)"
log_file="${tmpdir}/browser-adapter.log"
server_pid=""

cleanup() {
  if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
    kill "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
  rm -rf "${tmpdir}"
}
trap cleanup EXIT

pushd "${repo_root}" >/dev/null

cargo run -p traverse-cli-rs -- browser-adapter serve --bind 127.0.0.1:0 >"${log_file}" 2>&1 &
server_pid=$!

for _ in $(seq 1 200); do
  if grep -q "local browser adapter listening on " "${log_file}" 2>/dev/null; then
    break
  fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    cat "${log_file}" >&2
    echo "browser adapter exited before it reported a listening address" >&2
    exit 1
  fi
  sleep 0.05
done

base_url="$(grep -oE 'http://[^[:space:]]+' "${log_file}" | tail -n1)"
if [[ -z "${base_url}" ]]; then
  cat "${log_file}" >&2
  echo "failed to read browser adapter listening address" >&2
  exit 1
fi

python3 - "${base_url}" <<'PY'
import http.client
import json
import sys
import urllib.parse

base_url = sys.argv[1]
parsed = urllib.parse.urlparse(base_url)
connection = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=10)


def request(method, path, body=None, headers=None):
    connection.request(method, path, body=body, headers=headers or {})
    response = connection.getresponse()
    payload = response.read()
    return response.status, response.reason, dict(response.getheaders()), payload


create_payload = json.dumps(
    {
        "subscription_request": {
            "kind": "browser_runtime_subscription_request",
            "schema_version": "1.0.0",
            "governing_spec": "013-browser-runtime-subscription",
            "request_id": "expedition-plan-request-001",
        }
    }
).encode()

status, _, headers, payload = request(
    "POST",
    "/local/browser-subscriptions",
    body=create_payload,
    headers={"Content-Type": "application/json"},
)
assert status == 201, status
assert headers["Content-Type"].startswith("application/json")
created = json.loads(payload.decode())
assert created["kind"] == "local_browser_subscription_created"
assert created["schema_version"] == "1.0.0"
assert created["governing_spec"] == "019-local-browser-adapter-transport"
assert created["request_id"] == "expedition-plan-request-001"
assert created["execution_id"] == "exec_expedition-plan-request-001"
assert created["stream_url"] == "/local/browser-subscriptions/lbs_0001/stream"

status, _, headers, payload = request(
    "GET",
    created["stream_url"],
    headers={"Accept": "text/event-stream"},
)
assert status == 200, status
assert headers["Content-Type"].startswith("text/event-stream")

events = []
for chunk in payload.decode().strip().split("\n\n"):
    if not chunk:
        continue
    event_type = None
    event_data = None
    for line in chunk.splitlines():
        if line.startswith("event: "):
            event_type = line.removeprefix("event: ")
        elif line.startswith("data: "):
            event_data = json.loads(line.removeprefix("data: "))
    events.append((event_type, event_data))

assert [event for event, _ in events] == ["traverse_message"] * len(events)
message_variants = [next(iter(message.keys())) for _, message in events]
assert message_variants[0] == "Lifecycle"
assert message_variants[-1] == "Lifecycle"
assert "TraceArtifact" in message_variants
assert "StreamTerminal" in message_variants
assert "State" in message_variants

def inner(message, variant):
    return message[variant]

lifecycle_message = inner(events[0][1], "Lifecycle")
assert lifecycle_message["kind"] == "browser_runtime_subscription_lifecycle"
assert lifecycle_message["status"] == "subscription_established"

trace_message = inner(next(message for _, message in events if "TraceArtifact" in message), "TraceArtifact")
assert trace_message["kind"] == "browser_runtime_subscription_trace_artifact"

terminal_message = inner(next(message for _, message in events if "StreamTerminal" in message), "StreamTerminal")
assert terminal_message["kind"] == "browser_runtime_subscription_terminal"

status, _, _, payload = request(
    "POST",
    "/local/browser-subscriptions",
    body=json.dumps(
        {
            "subscription_request": {
                "kind": "browser_runtime_subscription_request",
                "schema_version": "1.0.0",
                "governing_spec": "013-browser-runtime-subscription",
            }
        }
    ).encode(),
    headers={"Content-Type": "application/json"},
)
assert status == 400, status
invalid = json.loads(payload.decode())
assert invalid["kind"] == "local_browser_subscription_setup_error"
assert invalid["code"] == "invalid_request"

status, _, _, payload = request(
    "GET",
    "/local/browser-subscriptions/lbs_9999/stream",
    headers={"Accept": "text/event-stream"},
)
assert status == 404, status
missing = json.loads(payload.decode())
assert missing["kind"] == "local_browser_subscription_stream_error"
assert missing["code"] == "not_found"
PY

popd >/dev/null

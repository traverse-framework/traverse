#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
manifest_path="${repo_root}/examples/expedition/registry-bundle/manifest.json"
request_path="${repo_root}/examples/expedition/runtime-requests/plan-expedition.json"

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

trace_path="${tmpdir}/plan-expedition-trace.json"
invalid_manifest_path="${tmpdir}/invalid-manifest.json"

pushd "${repo_root}" >/dev/null

register_output="$(cargo run -p traverse-cli-rs -- bundle register "${manifest_path}")"
printf '%s\n' "${register_output}"

grep -q "registered_capabilities: 6" <<<"${register_output}"
grep -q "registered_events: 5" <<<"${register_output}"
grep -q "registered_workflows: 1" <<<"${register_output}"

execution_output="$(cargo run -p traverse-cli-rs -- expedition execute "${request_path}" --trace-out "${trace_path}")"
printf '%s\n' "${execution_output}"

grep -q "status: completed" <<<"${execution_output}"
grep -q "recommended_route_style: conservative-alpine-push" <<<"${execution_output}"
grep -q "trace_path: ${trace_path}" <<<"${execution_output}"

trace_output="$(cargo run -p traverse-cli-rs -- trace inspect "${trace_path}")"
printf '%s\n' "${trace_output}"

grep -q "result_status: completed" <<<"${trace_output}"
grep -q "selected_capability_id: expedition.planning.plan-expedition" <<<"${trace_output}"
grep -q "terminal_transition: completed -> ready (execution_closed)" <<<"${trace_output}"

python3 - "${manifest_path}" "${invalid_manifest_path}" <<'PY'
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2])
payload = json.loads(source.read_text())
payload["workflows"][0]["path"] = "missing/plan-expedition/workflow.json"
target.write_text(json.dumps(payload, indent=2) + "\n")
PY

set +e
invalid_output="$(cargo run -p traverse-cli-rs -- bundle register "${invalid_manifest_path}" 2>&1)"
invalid_status=$?
set -e

if [[ ${invalid_status} -eq 0 ]]; then
  echo "expected bundle registration with a missing workflow artifact to fail" >&2
  exit 1
fi

grep -q "missing artifact file" <<<"${invalid_output}"

popd >/dev/null

#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
request_path="${repo_root}/examples/expedition/runtime-requests/plan-expedition.json"

pushd "${repo_root}" >/dev/null

output="$(cargo run -p traverse-cli-rs -- expedition execute "${request_path}")"
printf '%s\n' "${output}"

grep -q "capability_id: expedition.planning.plan-expedition" <<<"${output}"
grep -q "status: completed" <<<"${output}"
grep -q "recommended_route_style: conservative-alpine-push" <<<"${output}"

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

python3 - "${request_path}" "${tmpdir}/invalid-plan-expedition.json" <<'PY'
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2])
payload = json.loads(source.read_text())
payload["input"].pop("planning_intent", None)
target.write_text(json.dumps(payload, indent=2) + "\n")
PY

set +e
invalid_output="$(cargo run -p traverse-cli-rs -- expedition execute "${tmpdir}/invalid-plan-expedition.json" 2>&1)"
invalid_status=$?
set -e

if [[ ${invalid_status} -eq 0 ]]; then
  echo "expected expedition execution with invalid input to fail" >&2
  exit 1
fi

grep -q "runtime execution failed" <<<"${invalid_output}"
grep -q "runtime request input does not satisfy the selected capability input contract" <<<"${invalid_output}"

popd >/dev/null

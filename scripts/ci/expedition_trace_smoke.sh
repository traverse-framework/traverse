#!/usr/bin/env bash

set -euo pipefail

repo_root="${TRAVERSE_REPO_ROOT:-$(pwd)}"
request_path="${repo_root}/examples/expedition/runtime-requests/plan-expedition.json"

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

trace_path="${tmpdir}/plan-expedition-trace.json"

pushd "${repo_root}" >/dev/null

execution_output="$(cargo run -p traverse-cli-rs -- expedition execute "${request_path}" --trace-out "${trace_path}")"
printf '%s\n' "${execution_output}"

test -f "${trace_path}"
grep -q "trace_path: ${trace_path}" <<<"${execution_output}"

inspect_output="$(cargo run -p traverse-cli-rs -- trace inspect "${trace_path}")"
printf '%s\n' "${inspect_output}"

grep -q "trace_id: trace_exec_expedition-plan-request-001" <<<"${inspect_output}"
grep -q "result_status: completed" <<<"${inspect_output}"
grep -q "selected_capability_id: expedition.planning.plan-expedition" <<<"${inspect_output}"

printf '{"trace_id":true}\n' > "${tmpdir}/invalid-trace.json"

set +e
invalid_output="$(cargo run -p traverse-cli-rs -- trace inspect "${tmpdir}/invalid-trace.json" 2>&1)"
invalid_status=$?
set -e

if [[ ${invalid_status} -eq 0 ]]; then
  echo "expected malformed trace inspection to fail" >&2
  exit 1
fi

grep -q "failed to parse runtime trace" <<<"${invalid_output}"

popd >/dev/null

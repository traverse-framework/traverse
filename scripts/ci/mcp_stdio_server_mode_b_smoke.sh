#!/usr/bin/env bash

# Spec 520 / Spec 080 Mode B: prepare a host-owned verified cache from public
# registry refs, then serve stdio MCP from that cache only. No expedition
# checkout and no App-Refs materialize rewrite.

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
work_dir=$(mktemp -d)
cache_dir="${work_dir}/verified-cache"
state_path="${work_dir}/synced-state.json"
stdout_log=$(mktemp)
stderr_log=$(mktemp)
prepare_log=$(mktemp)
absent_stdout_log=$(mktemp)
absent_stderr_log=$(mktemp)
absent_cache_dir=$(mktemp -d)

cleanup() {
  rm -f "${stdout_log}" "${stderr_log}" "${prepare_log}" "${absent_stdout_log}" "${absent_stderr_log}"
  rm -rf "${work_dir}" "${absent_cache_dir}"
}
trap cleanup EXIT

wasm="${repo_root}/examples/core-normalize-participants/artifacts/core-normalize-participants.wasm"
contract="${repo_root}/examples/core-normalize-participants/contract.json"
if [[ ! -f "${wasm}" || ! -f "${contract}" ]]; then
  echo "Missing Mode B kit fixture under examples/core-normalize-participants." >&2
  exit 1
fi

python3 - "${state_path}" "${wasm}" "${contract}" <<'PY'
import hashlib, json, sys
from pathlib import Path

state_path, wasm_path, contract_path = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
wasm = wasm_path.read_bytes()
contract = contract_path.read_bytes()

def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()

record = {
    "namespace": "core",
    "id": "core.normalize-participants",
    "version": "1.1.0",
    "digest": digest(wasm),
    "artifact_url": f"file://{wasm_path}",
    "contract_digest": digest(contract),
    "contract_url": f"file://{contract_path}",
    "deprecated": False,
    "summary": "Normalize raw participants into canonical records.",
    "description": "Mode B prepare-cache smoke fixture.",
    "use_cases": [{"scenario": "Resolve extracted names and emails to workspace members."}],
    "service_type": "stateless",
    "permitted_targets": ["wasm"],
    "lifecycle": "active",
    "provenance": None,
}
state = {
    "schema_version": "1.0.0",
    "workspace_id": "mode-b-smoke",
    "state_scope": "public_registry_synced",
    "source_repo": "traverse-framework/registry",
    "release_tag": "index-v1",
    "index_version": 1,
    "generated_at": "2026-09-11T00:00:00Z",
    "source_commit": None,
    "synced_at": "2026-09-11T00:00:00Z",
    "record_count": 1,
    "validation_status": "valid",
    "governing_spec": "055-registry-sync",
    "capabilities": [record],
    "events": [],
}
Path(state_path).write_text(json.dumps(state), encoding="utf-8")
PY

cargo run -p traverse-mcp -- prepare-cache \
  --synced-state "${state_path}" \
  --cache "${cache_dir}" \
  --ref 'core/core.normalize-participants@=1.1.0' \
  --json >"${prepare_log}"

grep -q '"kind":"mcp_mode_b_prepare_cache"' "${prepare_log}"
grep -q '"governing_spec":"080-embedded-registry-cache"' "${prepare_log}"
grep -q '"cache_prepared":true' "${prepare_log}"
grep -q '"id":"core.normalize-participants"' "${prepare_log}"
if grep -q "${cache_dir}" "${prepare_log}"; then
  echo "Mode B prepare evidence must not echo the cache path." >&2
  exit 1
fi

kit_id="core.normalize-participants"
kit_version="1.1.0"
inline_request=$(python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)))' \
  <"${repo_root}/examples/core-normalize-participants/runtime-requests/uc01-mixed-match.json")

printf '%s\n' \
  '{"command":"describe_server"}' \
  '{"command":"list_entrypoints"}' \
  "{\"command\":\"execute_entrypoint\",\"entrypoint_kind\":\"capability\",\"id\":\"${kit_id}\",\"version\":\"${kit_version}\",\"request\":${inline_request}}" \
  '{"command":"shutdown"}' \
  | cargo run -p traverse-mcp -- stdio --cache "${cache_dir}" \
      >"${stdout_log}" 2>"${stderr_log}"

grep -q '"mode":"verified_public"' "${stdout_log}"
grep -q '"kind":"host_verified_public_registry"' "${stdout_log}"
grep -q "\"id\":\"${kit_id}\"" "${stdout_log}"
grep -q '"kind":"mcp_stdio_server_entrypoint_execution"' "${stdout_log}"
grep -q '"status":"completed"' "${stdout_log}"
grep -q '"digest_matches_public_state":true' "${stdout_log}"

if grep -q 'expedition' "${stdout_log}"; then
  echo "Mode B output must not reference the expedition catalog." >&2
  exit 1
fi

set +e
printf '%s\n' '{"command":"describe_server"}' \
  | cargo run -p traverse-mcp -- stdio --cache "${absent_cache_dir}" \
      >"${absent_stdout_log}" 2>"${absent_stderr_log}"
absent_status=$?
set -e

if [[ ${absent_status} -eq 0 ]]; then
  echo "Expected Mode B to fail closed without prepared verified state." >&2
  exit 1
fi
grep -q '"code":"registry_sync_missing"' "${absent_stderr_log}"
test ! -s "${absent_stdout_log}"

echo "MCP stdio server Mode B smoke passed."

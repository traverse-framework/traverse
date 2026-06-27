#!/usr/bin/env bash

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
manifest_path="${repo_root}/examples/applications/expedition-readiness/app.manifest.json"
workspace_id="downstream-local"
tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

validate_json="${tmpdir}/app-validate.json"
register_json="${tmpdir}/app-register.json"
runtime_json="${tmpdir}/runtime-load.json"

cargo run --quiet --manifest-path "${repo_root}/Cargo.toml" -p traverse-cli -- \
  app validate \
  --manifest "${manifest_path}" \
  --json > "${validate_json}"

python3 - "${validate_json}" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))
assert data["status"] == "validated"
assert data["app_id"] == "expedition.readiness"
assert data["app_version"] == "1.0.0"
assert "046-public-cli-app-registration" in data["governing_specs"]
assert {"cli", "http_json", "mcp"}.issubset(set(data["public_surfaces"]))
assert data["digest_verification"][0]["status"] == "verified"
assert data["model_readiness"][0]["status"] == "declared"
assert "expedition.planning.plan-expedition" in data["workflow_ids"]
PY

(
  cd "${tmpdir}"
  cargo run --quiet --manifest-path "${repo_root}/Cargo.toml" -p traverse-cli -- \
    app register \
    --manifest "${manifest_path}" \
    --workspace "${workspace_id}" \
    --json > "${register_json}"
)

python3 - "${register_json}" "${tmpdir}" "${workspace_id}" <<'PY'
import json
import pathlib
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))
workspace_root = pathlib.Path(sys.argv[2])
workspace_id = sys.argv[3]
assert data["status"] == "registered"
assert data["workspace_id"] == workspace_id
assert data["state_scope"] == "workspace_persisted"
assert data["app_id"] == "expedition.readiness"
assert data["components"][0]["wasm_digest"].startswith("sha256:")
assert data["workflows"][0]["workflow_digest"].startswith("sha256:")
state_path = workspace_root / data["state_path"]
assert state_path.is_file(), state_path
persisted = json.load(open(state_path, encoding="utf-8"))
assert persisted["status"] == "registered"
assert persisted["workspace_id"] == workspace_id
assert persisted["registration_fingerprint"]["app_id"] == "expedition.readiness"
PY

cargo run --quiet --manifest-path "${repo_root}/Cargo.toml" -p traverse-runtime \
  --example load_workspace_app_state -- \
  "${tmpdir}" \
  "${workspace_id}" > "${runtime_json}"

python3 - "${runtime_json}" "${workspace_id}" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))
workspace_id = sys.argv[2]
assert data["status"] == "loaded"
assert data["workspace_id"] == workspace_id
assert data["capability_count"] >= 1
assert data["workflow_count"] >= 1
assert "expedition.planning.validate-team-readiness" in data["capability_ids"]
assert "expedition.planning.plan-expedition" in data["workflow_ids"]
PY

echo "downstream public app registration smoke passed."

#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
default_repo_root="$(cd "${script_dir}/../.." && pwd)"
repo_root="${TRAVERSE_REPO_ROOT:-${default_repo_root}}"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

manifest_path="${tmp_dir}/governed-artifacts-manifest.json"

# Build a synthetic bundle manifest that includes every governed artifact under
# contracts/ and workflows/, excluding the top-level drafts/ quarantine.
python3 - "${repo_root}" "${manifest_path}" <<'PY'
import json
import sys
from pathlib import Path

repo_root = Path(sys.argv[1])
manifest_path = Path(sys.argv[2])

violations: list[dict[str, str]] = []

def record(code: str, path: str, message: str) -> None:
    violations.append({"violation_code": code, "path": path, "message": message})

def load_json(path: Path):
    try:
        return json.loads(path.read_text())
    except Exception as exc:
        record("invalid_json", str(path), f"failed to parse JSON: {exc}")
        return None

def is_quarantined(path: Path) -> bool:
    # drafts/ is a top-level quarantine directory. Ignore it entirely in CI-time validation.
    return "drafts" in path.parts

capabilities = []
events = []

contracts_root = repo_root / "contracts"
if contracts_root.is_dir():
    for path in sorted(contracts_root.rglob("contract.json")):
        if is_quarantined(path):
            continue
        obj = load_json(path)
        if obj is None:
            continue
        kind = obj.get("kind")
        if kind == "capability_contract":
            target = capabilities
        elif kind == "event_contract":
            target = events
        else:
            record("invalid_kind", str(path), f"unsupported contract kind: {kind!r}")
            continue

        cid = obj.get("id")
        ver = obj.get("version")
        if not cid:
            record("missing_required_field", f"{path}:$.id", "contract id is required")
            continue
        if not ver:
            record("missing_required_field", f"{path}:$.version", "contract version is required")
            continue
        target.append({"id": cid, "version": ver, "path": str(path)})

workflows = []
workflows_root = repo_root / "workflows"
if workflows_root.is_dir():
    for path in sorted(workflows_root.rglob("workflow.json")):
        if is_quarantined(path):
            continue
        obj = load_json(path)
        if obj is None:
            continue
        wid = obj.get("id")
        wver = obj.get("version")
        if not wid:
            record("missing_required_field", f"{path}:$.id", "workflow id is required")
            continue
        if not wver:
            record("missing_required_field", f"{path}:$.version", "workflow version is required")
            continue
        workflows.append({"id": wid, "version": wver, "path": str(path)})

if violations:
    payload = {"status": "failed", "violations": violations}
    print(json.dumps(payload, indent=2, sort_keys=True))
    sys.exit(2)

manifest = {
    "bundle_id": "traverse.ci.governed-artifacts",
    "version": "0.0.0",
    "scope": "private",
    "capabilities": capabilities,
    "events": events,
    "workflows": workflows,
}
manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
PY

# Run the registration-time gate over the full local artifact tree (which includes
# referential integrity checks like capability -> event references and workflow graph validity).
cargo run -p traverse-cli -- bundle register "${manifest_path}"

echo "Contractual enforcement gate passed."

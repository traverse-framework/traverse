#!/usr/bin/env python3
"""Best-effort Cursor pre-tool guard (#1877). Exit 2 to block."""
import json, re, sys
raw = sys.stdin.read()
try:
    data = json.loads(raw) if raw.strip() else {}
except Exception:
    raise SystemExit(0)
inp = data.get("tool_input") or data.get("input") or data
cmd = inp.get("command") or ""
path = inp.get("path") or inp.get("file_path") or ""
if cmd and re.search(r"\bgit\s+(commit|push)\b", cmd) and re.search(r"(--no-verify|\s-n\b)", cmd):
    print("Traverse guard: refusing --no-verify", file=sys.stderr)
    raise SystemExit(2)
for p in ("specs/governance/approved-specs.json", ".governance-version", "scripts/ci/coverage_gate.sh"):
    if path.endswith(p) or path == p:
        print(f"Traverse guard: refusing edit to {p}", file=sys.stderr)
        raise SystemExit(2)
raise SystemExit(0)

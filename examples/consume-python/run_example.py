#!/usr/bin/env python3
"""Run a governed Traverse WASM capability from Python via `traverse-cli`.

There is no native Python SDK for Traverse yet: this shells out to the real
`traverse-cli` binary using the documented `capability-package execute`
command (see docs/cli-reference.md) rather than embedding Wasmtime directly.
"""
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST = REPO_ROOT / "examples/capabilities/expedition-intent-agent/manifest.json"
REQUEST = REPO_ROOT / "examples/capabilities/runtime-requests/interpret-expedition-intent.json"


def main() -> int:
    for path in (MANIFEST, REQUEST):
        if not path.is_file():
            print(f"error: expected file not found: {path}", file=sys.stderr)
            return 1

    command = [
        "cargo",
        "run",
        "-p",
        "traverse-cli-rs",
        "--",
        "capability-package",
        "execute",
        str(MANIFEST),
        str(REQUEST),
    ]
    result = subprocess.run(command, cwd=REPO_ROOT)
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())

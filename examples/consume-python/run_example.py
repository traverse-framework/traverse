#!/usr/bin/env python3
"""Honest Python consume path: shell out to traverse-cli (no native SDK)."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def resolve_cli(root: Path) -> list[str]:
    """Prefer an installed binary; otherwise use cargo run against this checkout."""
    override = os.environ.get("TRAVERSE_CLI", "").strip()
    if override:
        return [override]

    which = shutil.which("traverse-cli")
    if which:
        return [which]

    return ["cargo", "run", "-q", "-p", "traverse-cli-rs", "--"]


def main() -> int:
    root = repo_root()
    manifest = root / "examples/capabilities/expedition-intent-agent/manifest.json"
    request = (
        root
        / "examples/capabilities/runtime-requests/interpret-expedition-intent.json"
    )

    if not manifest.is_file():
        print(f"missing manifest: {manifest}", file=sys.stderr)
        return 1
    if not request.is_file():
        print(f"missing request: {request}", file=sys.stderr)
        return 1

    cli = resolve_cli(root)
    cmd = [
        *cli,
        "capability-package",
        "execute",
        str(manifest),
        str(request),
    ]

    print("running:", " ".join(cmd), file=sys.stderr)
    result = subprocess.run(cmd, cwd=root, check=False)
    return int(result.returncode)


if __name__ == "__main__":
    raise SystemExit(main())

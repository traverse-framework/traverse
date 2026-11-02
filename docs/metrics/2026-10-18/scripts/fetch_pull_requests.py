#!/usr/bin/env python3
"""Read every pull request in traverse-framework/traverse with ``gh``.

This is a read-only collection script. It writes ``prs.json`` only inside
``--output``. It does not update the committed snapshot.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from common import PULL_REQUEST_QUERY, prepare_output, write_json


def run_gh(args: list[str]) -> dict:
    completed = subprocess.run(args, capture_output=True, text=True)
    if completed.returncode != 0:
        detail = (completed.stderr or "").strip().splitlines()
        tail = detail[-1] if detail else "gh api graphql failed"
        raise SystemExit(f"gh exited {completed.returncode}: {tail}")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"gh did not return JSON: {exc}") from exc


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    output_dir = prepare_output(args.output)
    collected: list[dict] = []
    cursor = None
    while True:
        command = ["gh", "api", "graphql", "-f", "query=" + PULL_REQUEST_QUERY]
        if cursor:
            command.extend(["-f", "c=" + cursor])
        payload = run_gh(command)
        page = payload["data"]["repository"]["pullRequests"]
        collected.extend(page["nodes"])
        print(len(collected), page["totalCount"], file=sys.stderr)
        if not page["pageInfo"]["hasNextPage"]:
            break
        cursor = page["pageInfo"]["endCursor"]
    destination = write_json(output_dir, "prs.json", collected)
    print(f"wrote {destination}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

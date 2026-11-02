#!/usr/bin/env python3
"""Read check suites for pull request commits with ``gh``.

Input is a ``prs.json`` dump. The script writes ``checkruns.json`` only inside
``--output``. The published first-run failure split does not read this file.
It is the GraphQL check-suite query used alongside the 18 October 2026 pull.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from common import load_json, prepare_output, write_json


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    output_dir = prepare_output(args.output)
    oids: list[str] = []
    for pull in load_json(args.input):
        for node in pull["commits"]["nodes"]:
            if node["commit"]["statusCheckRollup"]:
                oids.append(node["commit"]["oid"])
    unique = list(dict.fromkeys(oids))
    found: dict[str, dict] = {}
    for offset in range(0, len(unique), 25):
        batch = unique[offset : offset + 25]
        fields = []
        for index, oid in enumerate(batch):
            fields.append(
                f'c{index}:object(oid:"{oid}"){{...on Commit{{oid checkSuites(first:15){{nodes{{workflowRun{{workflow{{name}} event runNumber}} checkRuns(first:60){{nodes{{name conclusion startedAt}}}}}}}}}}}}'
            )
        query = (
            'query{repository(owner:"traverse-framework",name:"traverse"){'
            + "".join(fields)
            + "}}"
        )
        completed = subprocess.run(
            ["gh", "api", "graphql", "-f", "query=" + query],
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            detail = (completed.stderr or "").strip().splitlines()
            tail = detail[-1] if detail else "gh api graphql failed"
            raise SystemExit(f"gh exited {completed.returncode}: {tail}")
        payload = json.loads(completed.stdout)
        for value in payload["data"]["repository"].values():
            if value:
                found[value["oid"]] = value
        print(len(found), file=sys.stderr)
    destination = write_json(output_dir, "checkruns.json", found)
    print(f"wrote {destination} count={len(found)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Apply the agent-marker rule to a pull request dump.

``--input`` is a JSON list in the shape written by ``fetch_pull_requests.py``.
The classification file is written only under ``--output``.
"""

from __future__ import annotations

import argparse
import collections
import sys
from pathlib import Path

from common import classify, load_json, prepare_output, write_json


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    pulls = load_json(args.input)
    classified = {}
    counts: collections.Counter = collections.Counter()
    for pull in pulls:
        kind, evidence = classify(pull)
        classified[str(pull["number"])] = [kind, evidence]
        counts[kind] += 1
    output_dir = prepare_output(args.output)
    destination = write_json(output_dir, "classification.json", classified)
    print(
        f"CLASS total={len(pulls)} agent={counts['ai']} bot={counts['bot']} "
        f"unknown={counts['unknown']}"
    )
    print(f"wrote {destination}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Build a ``first_runs.json`` file from a pull dump and saved REST bodies.

``--check-runs-dir`` holds ``{sha}.json`` (check-runs) and ``{sha}.status.json``
(commit statuses) for each first-checked commit whose rollup failed. The script
writes ``first_runs.json`` only inside ``--output``.

The committed snapshot was built this way on 18 October 2026. Re-running it
needs those REST bodies, which are not in the repository. ``stats.py`` reads
the committed file directly.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from common import (
    classify,
    failure_names,
    first_checked,
    load_json,
    prepare_output,
    write_json,
)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--check-runs-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    check_dir = args.check_runs_dir.resolve()
    if not check_dir.is_dir():
        raise SystemExit(f"check-runs directory not found: {check_dir}")
    output_dir = prepare_output(args.output)
    rows = []
    for pull in load_json(args.input):
        kind, _evidence = classify(pull)
        sha, roll = first_checked(pull)
        row = {
            "n": pull["number"],
            "cls": kind,
            "state": pull["state"],
            "created": pull["createdAt"],
            "merged": pull["mergedAt"],
            "sha": sha,
            "roll": roll,
            "fails": [],
        }
        if sha and roll in ("FAILURE", "ERROR"):
            checks_path = check_dir / f"{sha}.json"
            status_path = check_dir / f"{sha}.status.json"
            if not checks_path.is_file() or not status_path.is_file():
                raise SystemExit(f"missing REST body for {sha} under {check_dir}")
            row["fails"] = failure_names(load_json(checks_path), load_json(status_path))
        rows.append(row)
    destination = write_json(output_dir, "first_runs.json", rows)
    print(f"wrote {destination} count={len(rows)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Count the 18 October 2026 pull request and first-run CI figures.

Reads ``../data/first_runs.json`` relative to this file (override with
``--data``). Prints the totals to stdout. Writes ``first_runs_cat.json`` only
when ``--output`` is set, and only inside that directory.
"""

from __future__ import annotations

import argparse
import collections
import sys
from datetime import datetime
from pathlib import Path

from common import (
    WINDOW_30D,
    WINDOW_60D,
    category,
    load_first_runs,
    parse_time,
    prepare_output,
    write_json,
)

WINDOWS = (
    ("30d", WINDOW_30D),
    ("60d", WINDOW_60D),
    ("all", datetime(2000, 1, 1, tzinfo=WINDOW_30D.tzinfo)),
)


def rate(count: int, total: int) -> str:
    if total == 0:
        return "n/a"
    return f"{100 * count / total:.1f}%"


def merged_in(rows: list[dict], start: datetime) -> list[dict]:
    selected = []
    for row in rows:
        if row["merged"] and parse_time(row["merged"]) >= start:
            selected.append(row)
    return selected


def summarize(rows: list[dict], label: str) -> collections.Counter:
    counted = [row for row in rows if row["roll"]]
    counts = collections.Counter(row["cat"] for row in counted)
    total = len(counted)
    failed = total - counts["pass"] - counts["pending"]
    metadata = counts["sa_only"] + counts["meta_only"]
    print(
        f"{label}: n={total} pass={counts['pass']} ({rate(counts['pass'], total)}) "
        f"fail={failed} [sa_only={counts['sa_only']} meta_only(other)={counts['meta_only']} "
        f"code={counts['code']}] pending={counts['pending']} "
        f"metadata={metadata}"
    )
    return counts


def headline(rows: list[dict]) -> None:
    classes = collections.Counter(row["cls"] for row in rows)
    print(
        f"HEADLINE total={len(rows)} agent={classes['ai']} bot={classes['bot']} "
        f"unknown={classes['unknown']} agent_share={rate(classes['ai'], len(rows))}"
    )
    for name, start in WINDOWS[:2]:
        chosen = [row for row in merged_in(rows, start) if row["cls"] == "ai"]
        counted = [row for row in chosen if row["roll"]]
        counts = collections.Counter(row["cat"] for row in counted)
        total = len(counted)
        failed = total - counts["pass"] - counts["pending"]
        metadata = counts["sa_only"] + counts["meta_only"]
        print(
            f"HEADLINE {name} agent={total} pass={counts['pass']} "
            f"rate={rate(counts['pass'], total)} fail={failed} "
            f"code={counts['code']} metadata={metadata} pending={counts['pending']}"
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--data",
        type=Path,
        default=None,
        help="first_runs.json to read (default: the committed snapshot)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="directory for first_runs_cat.json; omit to write nothing",
    )
    args = parser.parse_args(argv)
    rows = load_first_runs(args.data)
    for row in rows:
        row["cat"] = category(row)
    classes = collections.Counter(row["cls"] for row in rows)
    print(
        f"CLASS total={len(rows)} agent={classes['ai']} bot={classes['bot']} "
        f"unknown={classes['unknown']}"
    )
    for name, start in WINDOWS:
        merged = merged_in(rows, start)
        stamps = [row["merged"] for row in merged]
        print(
            f"== {name} merged={len(merged)} "
            f"no_checks={sum(1 for row in merged if not row['roll'])} "
            f"range={min(stamps) if stamps else 'n/a'} {max(stamps) if stamps else 'n/a'}"
        )
        for label in ("ai", "unknown", "bot"):
            summarize([row for row in merged if row["cls"] == label], f"  {label}")
        summarize(merged, "  all")
    headline(rows)
    if args.output is not None:
        output_dir = prepare_output(args.output)
        write_json(output_dir, "first_runs_cat.json", rows)
        print(f"wrote {output_dir / 'first_runs_cat.json'}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

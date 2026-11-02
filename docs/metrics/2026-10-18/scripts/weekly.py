#!/usr/bin/env python3
"""Print weekly first-run counts from the 18 October 2026 snapshot.

Reads the same ``first_runs.json`` as ``stats.py``. Writes nothing. Weeks start
on Monday in the fixed UTC-6 offset.
"""

from __future__ import annotations

import argparse
import collections
from datetime import timedelta
from pathlib import Path

from common import category, load_first_runs, parse_time


def week_start(stamp: str):
    day = parse_time(stamp).date()
    return day - timedelta(days=day.weekday())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, default=None)
    args = parser.parse_args(argv)
    rows = load_first_runs(args.data)
    for row in rows:
        row["cat"] = category(row)
    for key in ("created", "merged"):
        for group in ("ai", "all"):
            print(f"== weekly by {key} week (Monday start, UTC-6), {group}")
            buckets: dict = collections.defaultdict(list)
            for row in rows:
                if row["roll"] and row[key] and (group == "all" or row["cls"] == "ai"):
                    buckets[week_start(row[key])].append(row)
            print(
                "week\tn\tpass\tpass%\tfail\tsa_only\tmeta_other\tcode\t"
                "sa_any\tpass+sa_only%"
            )
            for start in sorted(buckets):
                grouped = buckets[start]
                counts = collections.Counter(row["cat"] for row in grouped)
                total = len(grouped)
                spec_alignment = sum(
                    "spec-alignment" in row["fails"] for row in grouped
                )
                failed = total - counts["pass"] - counts["pending"]
                passed_or_spec = counts["pass"] + counts["sa_only"]
                print(
                    f"{start}\t{total}\t{counts['pass']}\t{100 * counts['pass'] / total:.1f}\t"
                    f"{failed}\t{counts['sa_only']}\t{counts['meta_only']}\t{counts['code']}\t"
                    f"{spec_alignment}\t{100 * passed_or_spec / total:.1f}"
                )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

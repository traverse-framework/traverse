#!/usr/bin/env python3
"""Render non-blocking journal investigation markers for a workflow summary."""

import json
import pathlib
import sys


evidence_path = pathlib.Path(sys.argv[1])
data = json.loads(evidence_path.read_text(encoding="utf-8"))
environment = data["environment"]
append = data["append_latency_micros"]
recovery = data["restart_recovery_micros"]
disk = data["disk"]

markers = []
if append["p99"] > 25_000:
    markers.append("append p99 exceeds 25 ms")
if recovery > 1_000_000:
    markers.append("restart recovery exceeds 1 second")

status = "investigate" if markers else "within documented thresholds"
print(
    f"| {environment['os']}/{environment['arch']} | "
    f"{environment['storage_profile']} | {append['p50']} | {append['p95']} | "
    f"{append['p99']} | {recovery} | {data['replay']['events_per_second']} | "
    f"{disk['bytes_per_event_before_prune']} | {status} |"
)
if markers:
    print("Investigation markers: " + "; ".join(markers))

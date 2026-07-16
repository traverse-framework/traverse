#!/usr/bin/env python3
"""Create bounded fsync pressure beside the journal benchmark."""

import os
import pathlib
import signal
import sys


running = True


def stop(_signal: int, _frame: object) -> None:
    global running
    running = False


signal.signal(signal.SIGINT, stop)
signal.signal(signal.SIGTERM, stop)

root = pathlib.Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=True)
pressure_file = root / "journal-storage-pressure.bin"
block = b"p" * (1024 * 1024)

with pressure_file.open("wb", buffering=0) as handle:
    while running:
        handle.write(block)
        handle.flush()
        os.fsync(handle.fileno())
        if handle.tell() >= 256 * 1024 * 1024:
            handle.seek(0)

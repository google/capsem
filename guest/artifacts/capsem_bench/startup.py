"""CLI cold-start latency benchmarks."""

import subprocess
import time

from rich.table import Table
from rich.text import Text

from .helpers import console, drop_caches

STARTUP_COMMANDS = [
    ("python3", ["python3", "--version"]),
    ("node", ["node", "--version"]),
    ("claude", ["claude", "--version"]),
    ("gemini", ["gemini", "--version"]),
    ("codex", ["codex", "--version"]),
]
STARTUP_RUNS = 3


def time_command(cmd):
    """Run a command and return wall-clock duration in ms, or None on failure."""
    drop_caches()
    start = time.monotonic()
    try:
        subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                       timeout=60)
        elapsed = time.monotonic() - start
        return round(elapsed * 1000, 1)
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None


def startup_bench():
    """Time cold-start latency for key CLIs."""
    table = Table(title=Text(f"CLI Cold Start Latency  [{STARTUP_RUNS} runs each]"))
    table.add_column("Command", style="bold")
    table.add_column("Min (ms)", justify="right")
    table.add_column("Mean (ms)", justify="right")
    table.add_column("Max (ms)", justify="right")

    results = {"runs_per_command": STARTUP_RUNS, "commands": {}}

    for name, cmd in STARTUP_COMMANDS:
        timings = []
        for _ in range(STARTUP_RUNS):
            t = time_command(cmd)
            if t is not None:
                timings.append(t)

        if timings:
            entry = {
                "command": cmd,
                "timings_ms": timings,
                "min_ms": round(min(timings), 1),
                "mean_ms": round(sum(timings) / len(timings), 1),
                "max_ms": round(max(timings), 1),
            }
            results["commands"][name] = entry
            table.add_row(name, f"{entry['min_ms']}", f"{entry['mean_ms']}",
                          f"{entry['max_ms']}")
        else:
            results["commands"][name] = {"command": cmd, "error": "not found or timed out"}
            table.add_row(name, "-", "-", "-")

    console.print(table)
    return results

"""Runs already recorded: reading them back, and bounding what they occupy.

Separate from `runlog`, which writes one run. This is about the collection --
what is on disk, what a person can still open, and what has to go so the next
run has room. The two answer different questions and a single module carrying
both was past the size this package holds itself to.

Rotation prefers to keep the runs that crashed. A completed run had a terminal
showing its output at the time; a crashed one is exactly the case where that
output was lost with it, so it is the one somebody still wants.
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

from .config import GateConfig
from .fileactions import remove
from .harnessschema import RunLogConfig

_GB = 1024**3

def read(directory: Path, settings: RunLogConfig) -> list[dict]:
    """Every event in a run, in order."""
    source = directory / settings.events
    if not source.is_file():
        return []
    return [
        json.loads(line)
        for line in source.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def runs(config: GateConfig) -> list[Path]:
    """Every recorded run, newest first."""
    root = config.path(config.runlog.root)
    if not root.is_dir():
        return []
    return sorted(
        (entry for entry in root.iterdir() if entry.is_dir() and not entry.is_symlink()),
        key=lambda entry: entry.name,
        reverse=True,
    )


def rotate(config: GateConfig, *, keep: Path | None = None) -> list[Path]:
    """Drop the oldest runs until the policy is satisfied. Returns what went.

    A run with no `run.end` is one that crashed, and that is the run somebody
    still wants -- so completed runs are given up first, and an unfinished one
    is only dropped when nothing else is left to drop.

    `keep` is the run currently being written. Excluded outright rather than
    ranked last: a rotation that can delete the directory it is about to write
    into is a rotation with a bad day in it.
    """
    settings = config.runlog
    kept = [entry for entry in runs(config) if entry != keep]

    def surplus(remaining: list[Path]) -> bool:
        over_count = len(remaining) > settings.keep_runs
        over_bytes = sum(tree_size(entry) for entry in remaining) > settings.keep_bytes
        return over_count or over_bytes

    # Oldest first, and among equals the ones that finished.
    order = sorted(kept, key=lambda entry: (not finished(entry, settings), entry.name))
    removed: list[Path] = []
    for candidate in order:
        if not surplus(kept):
            break
        kept.remove(candidate)
        # Not `ignore_errors=True`: retention decides capacity from what it
        # reports here, so a run that refused to go and was counted as removed
        # makes every later decision against a number that is wrong.
        remove(candidate)
        removed.append(candidate)
    return removed


def finished(directory: Path, settings: RunLogConfig) -> bool:
    events = directory / settings.events
    if not events.is_file():
        return False
    return '"run.end"' in events.read_text(encoding="utf-8")


def free_gb(path: Path) -> float:
    return shutil.disk_usage(path).free / _GB


def tree_size(directory: Path) -> int:
    if not directory.is_dir():
        return 0
    return sum(entry.stat().st_size for entry in directory.rglob("*") if entry.is_file())


def head_revision(root: Path) -> str:
    head = root / ".git" / "HEAD"
    if not head.is_file():
        return ""
    reference = head.read_text(encoding="utf-8").strip()
    if not reference.startswith("ref: "):
        return reference
    resolved = root / ".git" / reference.removeprefix("ref: ")
    return resolved.read_text(encoding="utf-8").strip() if resolved.is_file() else ""

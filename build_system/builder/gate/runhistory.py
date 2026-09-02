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

import contextlib
import fcntl
import json
import os
import shutil
import subprocess
from collections.abc import Iterator
from contextlib import contextmanager
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
        json.loads(line) for line in source.read_text(encoding="utf-8").splitlines() if line.strip()
    ]


def runs(config: GateConfig) -> list[Path]:
    """Every recorded run, newest first."""
    root = config.path(config.runlog.root)
    if not root.is_dir():
        return []
    return sorted(
        (
            entry
            for entry in root.iterdir()
            if entry.is_dir()
            and not entry.is_symlink()
            and entry.name != config.runlog.source_archive_dir
        ),
        key=lambda entry: entry.name,
        reverse=True,
    )


def history_lock_path(config: GateConfig) -> Path:
    """Where the short-lived allocation lock lives.

    Deliberately not the machine lock. That one is held for the length of a
    gate, and a command opening its run log must not wait forty minutes for a
    directory -- `runs` would stop answering entirely.
    """
    return config.path(config.runlog.root) / config.runlog.history_lock


@contextmanager
def history_locked(config: GateConfig) -> Iterator[None]:
    """Serialize the operations that touch *another* run's directory.

    Allocation, rotation and repointing `latest` are the only three, and each
    is measured in milliseconds. Without this, a run opening under a tight
    retention cap could classify a live run as unfinished -- which is what a
    running gate looks like -- and delete the directory it was still writing.
    """
    path = history_lock_path(config)
    path.parent.mkdir(parents=True, exist_ok=True)
    handle = os.open(path, os.O_RDWR | os.O_CREAT, 0o644)
    try:
        fcntl.flock(handle, fcntl.LOCK_EX)
        yield
    finally:
        with contextlib.suppress(OSError):
            fcntl.flock(handle, fcntl.LOCK_UN)
        os.close(handle)


def hold_active(directory: Path, settings) -> int:
    """Flag a run as being written, for as long as this process lives.

    Taken before anything else can see the directory: the window between "it
    exists" and "it is marked live" is exactly when another process would
    classify it as a crashed run and rotate it away.
    """
    handle = os.open(directory / settings.active_marker, os.O_RDWR | os.O_CREAT, 0o644)
    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    return handle


def release_active(handle: int | None) -> None:
    """Give the flag back. Returns None, so the caller cannot double-close."""
    if handle is None:
        return
    with contextlib.suppress(OSError):
        fcntl.flock(handle, fcntl.LOCK_UN)
        os.close(handle)
    return


def point_latest(directory: Path, settings) -> None:
    """So `runs last` and a bug report have one path to name."""
    latest = directory.parent / settings.latest_link
    if latest.is_symlink() or latest.exists():
        latest.unlink()
    latest.symlink_to(directory.name)


def live(config: GateConfig) -> set[str]:
    """Runs another process is writing right now.

    A directory with no `run.end` is either a crashed run somebody wants to
    read or a run still being written; the difference is whether its lock file
    is held. Retention must never reach for the second kind.
    """
    running: set[str] = set()
    for directory in runs(config):
        marker = directory / config.runlog.active_marker
        if not marker.is_file():
            continue
        handle = os.open(marker, os.O_RDWR)
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(handle, fcntl.LOCK_UN)
        except BlockingIOError:
            running.add(directory.name)
        finally:
            os.close(handle)
    return running


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
    running = live(config)
    kept = [entry for entry in runs(config) if entry != keep and entry.name not in running]

    # Measured once, then kept as a running total. Re-walking every remaining
    # tree on every pass made the cost of starting a run quadratic in the runs
    # times the files they hold, and each re-walk answered the same question:
    # a directory that was not removed has not changed size.
    sizes = {entry: tree_size(entry) for entry in kept}
    total = sum(sizes.values())

    # Oldest first, and among equals the ones that finished.
    order = sorted(kept, key=lambda entry: (not finished(entry, settings), entry.name))
    removed: list[Path] = []
    for candidate in order:
        if len(kept) <= settings.keep_runs and total <= settings.keep_bytes:
            break
        kept.remove(candidate)
        total -= sizes[candidate]
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
    """The revision this run is of, or empty when there is no repository.

    Asked of git rather than parsed out of `.git` by hand. The hand-rolled
    version read `.git/HEAD` and then a loose ref, which fails on the two
    shapes a release is most likely cut from: in a linked worktree `.git` is a
    *file* pointing elsewhere, and a ref that has been packed has no loose file
    to read. Both returned "" -- a run recording no revision at all, silently.

    Empty stays the answer for a tree that is not a repository, because a
    tarball is a real way to receive source and it is not a failure.
    """
    try:
        found = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--verify", "HEAD"],
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return ""
    return found.stdout.strip() if found.returncode == 0 else ""

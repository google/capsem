"""Where faults go the moment they are found, and how they stop growing.

Two properties, and the second is why this is not one `open()` at a call site.

**It survives the run.** Line-buffered and fsynced per fault, because the run
being described may be killed -- and a report that only exists after a clean
exit is missing exactly when it is most wanted.

**It is bounded.** A run that trips one rule per file trips it thousands of
times, and an unbounded fault log on a machine that runs the gate daily is a
disk-full outage wearing a helpful name. Rotation is by size, oldest dropped
first, so the newest faults -- the ones describing the failure you are looking
at -- are the ones kept.
"""

from __future__ import annotations

import os
import time
from pathlib import Path

from .faults import Fault


class FaultLog:
    """An append-only, size-capped record of what a run did wrong."""

    def __init__(self, path: Path, *, max_bytes: int, keep: int) -> None:
        self.path = path
        self._max_bytes = max_bytes
        self._keep = keep
        path.parent.mkdir(parents=True, exist_ok=True)
        self._handle = path.open("a", buffering=1, encoding="utf-8")

    def __call__(self, fault: Fault) -> None:
        stamp = time.strftime("%H:%M:%S")
        line = f"{stamp} {fault.render()}\n"
        # Rotate *before* writing, not after exceeding. Rotating afterwards
        # leaves every generation one line over the cap, so the budget a disk
        # policy is written against is a lie by `keep + 1` lines.
        if self._handle.tell() + len(line.encode("utf-8")) > self._max_bytes:
            self._rotate()
        self._handle.write(line)
        # Per fault, not per close: the point is that a `kill -9` still leaves
        # the line describing what went wrong just before it.
        os.fsync(self._handle.fileno())

    def _rotate(self) -> None:
        """Shift the numbered generations down and start a new file.

        `keep` bounds total consumption at `max_bytes * (keep + 1)` exactly,
        which is the number a disk budget can be written against.
        """
        self._handle.close()
        oldest = self.path.with_suffix(f"{self.path.suffix}.{self._keep}")
        oldest.unlink(missing_ok=True)
        for generation in range(self._keep - 1, 0, -1):
            source = self.path.with_suffix(f"{self.path.suffix}.{generation}")
            if source.exists():
                source.replace(self.path.with_suffix(f"{self.path.suffix}.{generation + 1}"))
        if self._keep > 0:
            self.path.replace(self.path.with_suffix(f"{self.path.suffix}.1"))
        else:
            self.path.unlink(missing_ok=True)
        self._handle = self.path.open("a", buffering=1, encoding="utf-8")

    def close(self) -> None:
        self._handle.close()

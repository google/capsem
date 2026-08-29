"""One gate per machine, proven by the kernel.

A second `just test-clean` on the same machine is not a queueing inconvenience. The
first thing a run does is remove `$CAPSEM_HOME` and stop the service inside it,
so two runs means one deletes the other's home mid-flight and both report
failures that belong to neither.

`flock` rather than a pidfile, because a pidfile needs a staleness heuristic --
is this pid alive, is it still the same process, was the file left by a crash
-- and every one of those is a way to either wedge the machine or let two runs
start anyway. The kernel drops the lock when the fd closes, including when the
holder is killed, so there is nothing to clean up and nothing to get wrong.

Three things this has to get right, all of which `build_system/scripts/build/lib/exec_lock.sh` got
right in comments and nothing checked:

  the lockfile sits outside the tree the gate is about to wipe, or it is
  deleted while held and the next run locks a fresh inode

  the fd is not inherited by a launched daemon, or `capsem-service` holds the
  lock for as long as it lives and the next run blocks on a gate that finished
  hours ago -- a hang nobody attributes to a file descriptor

  contention names the holder instead of blocking mutely, because someone who
  left a run going in another terminal should be told that
"""

from __future__ import annotations

import fcntl
import json
import os
import socket
import time
from pathlib import Path

from .config import GateConfig
from .errors import GateError
from .lifecycle import Resource
from .lockschema import LockConfig


class ExclusiveLock(Resource, name="gate-lock"):
    """The machine, held for the length of one command.

    A `Resource`, so it is taken through `held(...)` with everything else and
    released in reverse -- which means an interrupted gate drops it rather
    than leaving the machine locked by a process that no longer exists.
    """

    def __init__(self, settings: LockConfig, *, purpose: str) -> None:
        self._settings = settings
        self._purpose = purpose
        self.path = Path(settings.path)
        self._record = Path(settings.holder_record)
        self._fd: int | None = None

    @classmethod
    def for_gate(cls, config: GateConfig, *, purpose: str) -> ExclusiveLock:
        """The one gate lock, resolved against the user rather than a tree.

        Worktrees and detached qualification prefixes share the machine state
        this protects. Resolving against either checkout gives each a distinct
        inode and makes the lock a convincing no-op.
        """
        settings = config.locks.gate
        return cls(
            settings.model_copy(
                update={
                    "path": str(Path(settings.path).expanduser()),
                    "holder_record": str(Path(settings.holder_record).expanduser()),
                }
            ),
            purpose=purpose,
        )

    def environment(self) -> dict[str, str]:
        """Tell every descendant that it is inside a run.

        flock cannot answer this from the child's side: the lock its own parent
        holds is indistinguishable from a stranger's, so a gate command started
        by anything the gate launched -- the pytest step, most of all -- waited
        out the full timeout for a lock that would never be released until it
        returned. Exported rather than global, so it dies with the lock.
        """
        return {self._settings.run_marker: self._purpose}

    @property
    def fileno(self) -> int:
        if self._fd is None:
            raise GateError("the gate lock is not held")
        return self._fd

    # -- Resource ----------------------------------------------------------

    def acquire(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(self.path, os.O_RDWR | os.O_CREAT, 0o644)
        # Python has defaulted to non-inheritable since PEP 446. Stated anyway,
        # so that adding `pass_fds` to a daemon launch is a visible decision
        # rather than a quiet reintroduction of the `3>&-` bug.
        os.set_inheritable(fd, False)

        try:
            self._take(fd)
        except BaseException:
            os.close(fd)
            raise

        self._fd = fd
        self._write_record()

    def release(self) -> None:
        """Drop the lock, if this instance ever held it.

        Teardown runs against whatever state a failure left behind, so being
        asked to release a lock that was never taken is expected.
        """
        if self._fd is None:
            return
        self._record.unlink(missing_ok=True)
        fcntl.flock(self._fd, fcntl.LOCK_UN)
        os.close(self._fd)
        self._fd = None

    # -- taking it ---------------------------------------------------------

    def _take(self, fd: int) -> None:
        """Try, then report who has it, then wait until the deadline."""
        settings = self._settings
        started = time.monotonic()
        deadline = started + settings.wait_timeout_seconds
        reported = False

        while True:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return
            except BlockingIOError:
                waited = time.monotonic() - started
                if not reported and waited >= settings.report_after_seconds:
                    print(f"waiting: {self._holder()}")
                    reported = True
                if time.monotonic() >= deadline:
                    raise GateError(
                        f"gave up after {waited:.0f}s waiting for the gate "
                        f"lock at {self.path}: {self._holder()}"
                    ) from None
                time.sleep(settings.poll_interval_seconds)

    def _holder(self) -> str:
        """Whoever has the lock, in as much detail as survives.

        The record is written after the lock is taken, so there is a window
        where it is absent -- and a killed writer can leave half of it. Neither
        may turn contention into a different error, so both fall back to
        saying the true and useful part.
        """
        try:
            record = json.loads(self._record.read_text(encoding="utf-8"))
            held_for = time.time() - float(record["started"])
            return (
                f"another gate holds it: {record['purpose']!r} "
                f"(pid {record['pid']} on {record['host']}, {held_for:.0f}s ago)"
            )
        except (OSError, ValueError, KeyError):
            return f"another gate holds it (no readable holder record at {self._record})"

    def _write_record(self) -> None:
        """Say who has it, for whoever arrives next.

        Advisory only. The lock is the kernel's; this is so a person can find
        out what to wait for, or what to stop.
        """
        self._record.parent.mkdir(parents=True, exist_ok=True)
        self._record.write_text(
            json.dumps(
                {
                    "pid": os.getpid(),
                    "purpose": self._purpose,
                    "started": time.time(),
                    "host": socket.gethostname(),
                }
            ),
            encoding="utf-8",
        )

"""Stopping the daemons a gate started, and knowing whether it worked.

The whole difficulty is that stopping nothing looks exactly like stopping
something. `stop_gate_pidfile` on a path no binary writes removes a file that
was never there and returns success -- which is how sixteen `capsem-service`
processes, each holding a `capsem-tray`, accumulated in a single day while
every run reported a clean shutdown.

The names and timeouts are `[pidfiles]` in `config/gate.toml`; the order is
part of the data, not a convention this module remembers.

Two rules follow, and both are enforced here rather than remembered:

  a pidfile this stops must be one some binary writes
      `tests/test_pidfile_cleanup_is_wired.py` checks that against the crates,
      so a typo in a filename fails a test instead of leaking a process.

  a process that will not die is a failure, not a warning
      SIGTERM, wait, SIGKILL, wait again -- and if it is still there, say so.
      Returning success here is what makes the leak invisible.

The gateway is stopped before the service on purpose: it owns the fixed
localhost port, and a gateway that outlives its service attaches the next
profile to a UDS pointing at a deleted run directory.
"""

from __future__ import annotations

import errno
import os
import signal
import time
from pathlib import Path

from . import config as gate_config
from .errors import GateError


def running(pid: int) -> bool:
    """Whether `pid` is a live process rather than an unreaped zombie.

    A zombie answers `kill -0` for as long as nobody waits on it, so treating
    that as "still running" makes the stop loop time out on a process that has
    already exited.
    """
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError as error:  # pragma: no cover - defensive
        if error.errno == errno.ESRCH:
            return False
        raise
    return not _is_zombie(pid)


def _is_zombie(pid: int) -> bool:
    status = Path(f"/proc/{pid}/stat")
    if status.is_file():  # Linux
        try:
            return status.read_text(encoding="utf-8").rsplit(")", 1)[1].split()[0] == "Z"
        except (OSError, IndexError):  # pragma: no cover - racing the exit
            return False
    # macOS has no /proc; ps is the portable answer and this path runs a
    # handful of times per gate.
    import subprocess

    state = subprocess.run(
        ["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True
    ).stdout.strip()
    return state.startswith("Z")


def _await_exit(pid: int, seconds: float, poll: float) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not running(pid):
            return True
        time.sleep(poll)
    return not running(pid)


def stop(pidfile: Path, settings: gate_config.PidfileConfig) -> None:
    """Stop the process a pidfile names, and remove the file.

    An absent pidfile is not an error -- the daemon may never have started --
    but a *named* process that survives both signals is.
    """
    pidfile = Path(pidfile)
    recorded = _recorded_pid(pidfile)

    if recorded is not None and running(recorded):
        os.kill(recorded, signal.SIGTERM)
        if not _await_exit(recorded, settings.term_wait_seconds, settings.poll_interval_seconds):
            os.kill(recorded, signal.SIGKILL)
            if not _await_exit(
                recorded, settings.kill_wait_seconds, settings.poll_interval_seconds
            ):
                raise GateError(
                    f"isolated asset gate {pidfile.name} process {recorded} did not exit"
                )

    pidfile.unlink(missing_ok=True)


def _recorded_pid(pidfile: Path) -> int | None:
    try:
        recorded = pidfile.read_text(encoding="utf-8").strip()
    except (OSError, ValueError):
        return None
    return int(recorded) if recorded.isdigit() else None


def stop_gate_service(run_dir: Path, settings: gate_config.PidfileConfig) -> None:
    """Stop everything a gate's isolated run directory started, in order."""
    for name in settings.names:
        stop(Path(run_dir) / name, settings)

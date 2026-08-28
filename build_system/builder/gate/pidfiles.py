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
      `build_system/tests/scripts/test_pidfile_cleanup_is_wired.py` checks that against the crates,
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


def running(pid: int, settings: gate_config.PidfileConfig) -> bool:
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
    return not _is_zombie(pid, settings.proc_stat_template)


#: `PROC_PIDTBSDINFO`, and room for the struct it fills. The flavour number is
#: a stable macOS ABI constant; the buffer is generous because the exact struct
#: size is a header detail and `proc_pidinfo` reports what it wrote.
_PROC_PIDTBSDINFO = 3
_PROC_INFO_BUFFER = 4096


def _is_zombie(pid: int, stat_template: str) -> bool:
    status = Path(stat_template.format(pid=pid))
    if status.is_file():  # Linux
        try:
            return status.read_text(encoding="utf-8").rsplit(")", 1)[1].split()[0] == "Z"
        except (OSError, IndexError):  # pragma: no cover - racing the exit
            return False
    # macOS has no /proc, and it cannot use `ps` either: `/bin/ps` is setuid
    # root, and a sandboxed process may not exec a setuid binary at all --
    # `PermissionError: [Errno 1] Operation not permitted: 'ps'`, which is how
    # the gate's own liveness check failed under the gate's own sandbox.
    #
    # `proc_pidinfo` answers the same question with a syscall. It reports a
    # state for a live process and fails outright for a zombie, which has no
    # BSD info left to report -- so "the caller could signal it but the kernel
    # will not describe it" is exactly the zombie case.
    return not _describable(pid)


def _describable(pid: int) -> bool:
    """Whether the kernel still has BSD process info for `pid`.

    False for a zombie. Deliberately not a state comparison: the constant for
    `SZOMB` is not exported anywhere Python can see, and the call failing is
    the more robust signal -- it is what the kernel does rather than a number
    this file would have to keep in step with a header.
    """
    import ctypes
    import ctypes.util

    library = ctypes.util.find_library("proc")
    if library is None:  # pragma: no cover - libproc is part of macOS
        return True
    buffer = ctypes.create_string_buffer(_PROC_INFO_BUFFER)
    written = ctypes.CDLL(library).proc_pidinfo(
        ctypes.c_int(pid),
        ctypes.c_int(_PROC_PIDTBSDINFO),
        ctypes.c_uint64(0),
        buffer,
        ctypes.c_int(len(buffer)),
    )
    return written > 0


def _await_exit(pid: int, seconds: float, settings: gate_config.PidfileConfig) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not running(pid, settings):
            return True
        time.sleep(settings.poll_interval_seconds)
    return not running(pid, settings)


def stop(pidfile: Path, settings: gate_config.PidfileConfig) -> None:
    """Stop the process a pidfile names, and remove the file.

    An absent pidfile is not an error -- the daemon may never have started --
    but a *named* process that survives both signals is.
    """
    pidfile = Path(pidfile)
    recorded = _recorded_pid(pidfile)

    if recorded is not None and running(recorded, settings):
        os.kill(recorded, signal.SIGTERM)
        if not _await_exit(recorded, settings.term_wait_seconds, settings):
            os.kill(recorded, signal.SIGKILL)
            if not _await_exit(recorded, settings.kill_wait_seconds, settings):
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

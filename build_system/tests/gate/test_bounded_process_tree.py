"""Direct-command timeouts must cross child-created process sessions."""

from __future__ import annotations

import contextlib
import os
import signal
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BOUNDED = ROOT / "build_system/scripts/ci/run-bounded-command.py"
SYSTEM_PYTHON = Path("/usr/bin/python3")


def _alive(pid: int) -> bool:
    result = subprocess.run(
        ["ps", "-p", str(pid), "-o", "stat="],
        check=False,
        capture_output=True,
        text=True,
    )
    return result.returncode == 0 and bool(result.stdout.strip())


def test_timeout_reaps_a_descendant_that_created_a_new_session(tmp_path: Path) -> None:
    child_pid = tmp_path / "detached-child"
    child = "import time; time.sleep(300)"
    leader = (
        "import subprocess,sys,time; "
        f"child=subprocess.Popen([sys.executable,'-c',{child!r}],start_new_session=True); "
        "open(sys.argv[1],'w').write(str(child.pid)); time.sleep(300)"
    )

    pid: int | None = None
    try:
        result = subprocess.run(
            [
                str(SYSTEM_PYTHON),
                str(BOUNDED),
                "--timeout-seconds",
                "5",
                "--grace-seconds",
                "0.2",
                "--",
                sys.executable,
                "-c",
                leader,
                str(child_pid),
            ],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        pid = int(child_pid.read_text())
        assert result.returncode == 124, result.stderr
        assert not _alive(pid)
    finally:
        if pid is None and child_pid.exists():
            pid = int(child_pid.read_text())
        if pid is not None and _alive(pid):
            with contextlib.suppress(ProcessLookupError):
                os.kill(pid, signal.SIGKILL)

"""Unambiguous command support shared by the in-VM diagnostics."""

from __future__ import annotations

import contextlib
import os
import signal
import subprocess
from pathlib import Path

import pytest

TESTS_OUTPUT_DIR = Path("/root/tests")


def _timeout_diagnostics() -> str:
    """Capture bounded guest load/process evidence while a command is stuck."""
    diagnostic_cmd = (
        "echo '--- /proc/loadavg ---'; cat /proc/loadavg 2>&1; "
        "echo '--- memory ---'; free -m 2>&1; "
        "echo '--- busiest processes ---'; "
        "ps -eo pid,ppid,stat,etime,pcpu,pmem,comm,args --sort=-pcpu 2>&1 | head -n 30"
    )
    try:
        result = subprocess.run(
            diagnostic_cmd,
            shell=True,
            capture_output=True,
            text=True,
            timeout=5,
        )
        return result.stdout + result.stderr
    except subprocess.TimeoutExpired:
        return "guest timeout diagnostics also exceeded 5s"


def run(cmd: str, timeout: float = 10) -> subprocess.CompletedProcess[str]:
    """Run a shell command with bounded diagnostics and process-group cleanup."""
    process = subprocess.Popen(
        cmd,
        shell=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        diagnostics = _timeout_diagnostics()
        try:
            os.killpg(process.pid, signal.SIGTERM)
            stdout, stderr = process.communicate(timeout=2)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            with contextlib.suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
        pytest.fail(
            f"command timed out after {timeout}s: {cmd}\n"
            f"STDOUT:\n{stdout}\nSTDERR:\n{stderr}\n{diagnostics}",
            pytrace=False,
        )
    return subprocess.CompletedProcess(cmd, process.returncode, stdout, stderr)

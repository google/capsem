"""Bounded argv-only execution for native cache adapters."""

from __future__ import annotations

import os
import signal
import subprocess
import time
from collections.abc import Callable

from .runtimemodels import RuntimeCommandResult

CommandRunner = Callable[[tuple[str, ...], int], RuntimeCommandResult]


def execute(argv: tuple[str, ...], timeout_seconds: int) -> RuntimeCommandResult:
    """Run one closed-stdin command and own its process group through timeout."""
    started = time.monotonic_ns()
    try:
        process = subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
    except FileNotFoundError as error:
        return _result(argv, 127, "", str(error), started)
    try:
        stdout, stderr = process.communicate(timeout=timeout_seconds)
        return _result(argv, process.returncode, stdout, stderr, started)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        stdout, stderr = process.communicate()
        return _result(argv, 124, stdout, f"{stderr}\ncommand timed out", started)


def _result(
    argv: tuple[str, ...], returncode: int, stdout: str, stderr: str, started: int
) -> RuntimeCommandResult:
    return RuntimeCommandResult(
        argv=argv,
        returncode=returncode,
        stdout=stdout.strip(),
        stderr=stderr.strip(),
        duration_ms=max(0, (time.monotonic_ns() - started) // 1_000_000),
    )

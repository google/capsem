"""Run one direct development command in a bounded process group.

This is deliberately separate from capsem-gate.  Gate steps use the timeout,
journal, resource, and resume contracts in ``config/gate.toml``; this wrapper
protects the focused commands developers run while diagnosing those steps.
"""

from __future__ import annotations

import argparse
import os
import signal
import subprocess
import sys
import time
from collections.abc import Sequence
from typing import Protocol

TIMEOUT_EXIT = 124


class _ProcessGroup(Protocol):
    pid: int

    def poll(self) -> int | None: ...

    def wait(self, timeout: float | None = None) -> int: ...


class _ForwardedSignal(Exception):
    """Leave the wait loop while retaining the signal that interrupted it."""

    def __init__(self, signum: int) -> None:
        super().__init__(signum)
        self.signum = signum


def _positive_seconds(value: str) -> float:
    seconds = float(value)
    if seconds <= 0:
        raise argparse.ArgumentTypeError("must be greater than zero")
    return seconds


def _nonnegative_seconds(value: str) -> float:
    seconds = float(value)
    if seconds < 0:
        raise argparse.ArgumentTypeError("must be zero or greater")
    return seconds


def _terminate_group(process: _ProcessGroup, grace_seconds: float) -> None:
    """Terminate every descendant in the command's dedicated process group."""
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        return

    deadline = time.monotonic() + grace_seconds
    while True:
        process.poll()  # reap the process-group leader as soon as it exits
        try:
            os.killpg(process.pid, 0)
        except (ProcessLookupError, PermissionError):
            return
        if time.monotonic() >= deadline:
            break
        time.sleep(min(0.02, max(0.0, deadline - time.monotonic())))

    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        return
    if process.poll() is None:
        process.wait()


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "run a direct development command with closed stdin and a bounded "
            "process-group lifetime"
        )
    )
    parser.add_argument("--timeout-seconds", required=True, type=_positive_seconds)
    parser.add_argument("--grace-seconds", default=10.0, type=_nonnegative_seconds)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    return parser


def run(argv: Sequence[str]) -> int:
    args = _parser().parse_args(argv)
    command = list(args.command)
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        _parser().error("a command is required after --")

    process = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
    )

    def interrupted(signum: int, _frame: object) -> None:
        raise _ForwardedSignal(signum)

    previous = {
        signum: signal.signal(signum, interrupted)
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
    }
    try:
        try:
            return process.wait(timeout=args.timeout_seconds)
        except subprocess.TimeoutExpired:
            print(
                f"bounded command timed out after {args.timeout_seconds:g}s; "
                f"terminating process group {process.pid}",
                file=sys.stderr,
            )
            _terminate_group(process, args.grace_seconds)
            return TIMEOUT_EXIT
        except _ForwardedSignal as caught:
            _terminate_group(process, args.grace_seconds)
            return 128 + caught.signum
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)
        _terminate_group(process, args.grace_seconds)


def main() -> int:
    return run(sys.argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())

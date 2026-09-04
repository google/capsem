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
from collections.abc import Sequence
from typing import Protocol, cast

from capsem_builder.gate import processgroup

TIMEOUT_EXIT = 124


def _contained_environment() -> dict[str, str]:
    """Route direct language tools through the repository cache policy."""
    inherited = dict(os.environ)
    raw_root = inherited.get("CAPSEM_REPOSITORY_ROOT")
    if not raw_root:
        return inherited
    from pathlib import Path

    root = Path(raw_root).resolve()
    if not (root / "config/cache.toml").is_file():
        return inherited

    from capsem_builder import gatelaunch

    selected = gatelaunch.contained_environment(root)
    gatelaunch.hold_environment(root)
    return {**inherited, **selected}


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


def _terminate_tree(process: _ProcessGroup, grace_seconds: float) -> None:
    """Terminate the command group and descendant-created sessions."""
    policy = processgroup.StopPolicy(
        grace_seconds=max(grace_seconds, 0.001),
        poll_seconds=0.02,
    )
    processgroup.terminate(
        cast(processgroup.ForegroundProcess, process),
        policy,
    )


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
        env=_contained_environment(),
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
            _terminate_tree(process, args.grace_seconds)
            return TIMEOUT_EXIT
        except _ForwardedSignal as caught:
            _terminate_tree(process, args.grace_seconds)
            return 128 + caught.signum
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)
        _terminate_tree(process, args.grace_seconds)


def main() -> int:
    return run(sys.argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())

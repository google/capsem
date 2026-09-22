"""Own, identify, and stop one foreground process tree."""

from __future__ import annotations

import os
import signal
import subprocess
import sys
import time
from contextlib import suppress
from dataclasses import dataclass
from typing import TypeAlias

from psutil import STATUS_ZOMBIE, NoSuchProcess
from psutil import Error as ProcessError
from psutil import Process as SystemProcess

from .errors import GateError

ForegroundProcess: TypeAlias = subprocess.Popen[str] | subprocess.Popen[bytes]
OwnedTree: TypeAlias = dict[int, SystemProcess]


@dataclass(frozen=True)
class StopPolicy:
    """Config-derived bounds for noticing cancellation and stopping a group."""

    grace_seconds: float
    poll_seconds: float
    refuse_survivors: bool = True
    """Whether a process outliving its command fails the run, or is reported."""

    @classmethod
    def from_execution(cls, execution) -> StopPolicy:
        return cls(
            grace_seconds=execution.cancellation_grace_seconds,
            poll_seconds=execution.cancellation_poll_seconds,
            refuse_survivors=not os.environ.get(execution.survivors_unenforced_when_set),
        )

    def __post_init__(self) -> None:
        if self.grace_seconds <= 0 or self.poll_seconds <= 0:
            raise ValueError("process stop policy durations must be positive")


def remember_descendants(pid: int, owned: OwnedTree) -> None:
    try:
        descendants = SystemProcess(pid).children(recursive=True)
    except NoSuchProcess:
        return
    for process in descendants:
        owned.setdefault(process.pid, process)


def refuse_descendants(process: ForegroundProcess, policy: StopPolicy, owned: OwnedTree) -> None:
    """Refuse a foreground leader that turned children into hidden daemons."""
    remember_descendants(process.pid, owned)
    descendants = tuple(owned.values())
    if not _group_exists(process.pid) and not _descendants_alive(descendants):
        return
    deadline = time.monotonic() + policy.poll_seconds
    while (
        _group_exists(process.pid) or _descendants_alive(descendants)
    ) and time.monotonic() < deadline:
        time.sleep(policy.poll_seconds)
    if not _group_exists(process.pid) and not _descendants_alive(descendants):
        return
    surviving = _surviving(descendants)
    terminate(process, policy, owned)
    if not policy.refuse_survivors:
        print(
            f"warning: {process.pid} exited leaving "
            + ("; ".join(surviving) if surviving else "its process group")
            + " behind; reaped rather than refused",
            file=sys.stderr,
            flush=True,
        )
        return
    raise GateError(
        f"foreground process {process.pid} exited while descendants remained; "
        "long-lived work must use Runner.launch"
        + (
            f" -- still running: {'; '.join(surviving)}"
            if surviving
            else " -- the process group outlived it with no descendant left to name"
        )
    )


def _surviving(descendants: tuple[SystemProcess, ...]) -> list[str]:
    alive: list[str] = []
    for process in descendants:
        try:
            if not process.is_running() or process.status() == STATUS_ZOMBIE:
                continue
            command = " ".join(process.cmdline()[:6]) or process.name()
        except ProcessError:
            continue
        alive.append(f"{process.pid} {command}")
    return alive


def terminate(
    process: ForegroundProcess, policy: StopPolicy, owned: OwnedTree | None = None
) -> None:
    """Stop a process group and every remembered descendant session."""
    tracked = {} if owned is None else owned
    remember_descendants(process.pid, tracked)
    descendants = tuple(tracked.values())
    _signal_descendants(descendants, signal.SIGTERM)
    _signal_group(process.pid, signal.SIGTERM)
    _reap_leader(process, policy.poll_seconds)
    if not _wait_for_owned(process.pid, descendants, policy):
        _signal_descendants(descendants, signal.SIGKILL)
        _signal_group(process.pid, signal.SIGKILL)
        _reap_leader(process, policy.grace_seconds)
        if not _wait_for_owned(process.pid, descendants, policy):
            raise GateError(
                f"owned process tree {process.pid} survived SIGKILL after {policy.grace_seconds:g}s"
            )


def _reap_leader(process: ForegroundProcess, timeout: float) -> None:
    with suppress(subprocess.TimeoutExpired):
        process.wait(timeout=timeout)


def _wait_for_owned(group: int, descendants: tuple[SystemProcess, ...], policy: StopPolicy) -> bool:
    deadline = time.monotonic() + policy.grace_seconds
    while (_group_exists(group) or _descendants_alive(descendants)) and time.monotonic() < deadline:
        time.sleep(policy.poll_seconds)
    return not _group_exists(group) and not _descendants_alive(descendants)


def _group_exists(group: int) -> bool:
    try:
        os.killpg(group, 0)
    except (ProcessLookupError, PermissionError):
        return False
    return True


def _signal_group(group: int, sent: signal.Signals) -> None:
    with suppress(ProcessLookupError, PermissionError):
        os.killpg(group, sent)


def _signal_descendants(descendants: tuple[SystemProcess, ...], sent: signal.Signals) -> None:
    for process in reversed(descendants):
        with suppress(NoSuchProcess):
            process.send_signal(sent)


def _descendants_alive(descendants: tuple[SystemProcess, ...]) -> bool:
    for process in descendants:
        try:
            if process.is_running() and process.status() != STATUS_ZOMBIE:
                return True
        except NoSuchProcess:
            continue
    return False

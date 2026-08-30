"""One foreground command, its descendants, and how they stop together.

Foreground work belongs to the gate for exactly as long as the action which
started it. A command that needs to outlive that action uses ``Runner.launch``;
everything else gets a fresh process group so cancellation never has to guess
at executable names or touch another developer's process.
"""

from __future__ import annotations

import os
import selectors
import signal
import subprocess
import sys
import time
from collections.abc import Callable, Mapping, Sequence
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import TypeAlias

from psutil import STATUS_ZOMBIE, NoSuchProcess
from psutil import Error as ProcessError
from psutil import Process as SystemProcess

from . import cancellation
from .errors import GateError

ForegroundProcess: TypeAlias = subprocess.Popen[str] | subprocess.Popen[bytes]
OwnedTree: TypeAlias = dict[int, SystemProcess]


@dataclass(frozen=True)
class StopPolicy:
    """Config-derived bounds for noticing cancellation and stopping a group."""

    grace_seconds: float
    poll_seconds: float
    refuse_survivors: bool = True
    """Whether a process outliving its command fails the run, or is reported.

    Reaped and named either way. See `[execution]
    survivors_unenforced_when_set` for which machines it fails on and why.
    """

    @classmethod
    def from_execution(cls, execution) -> StopPolicy:
        """Build the policy from `[execution]`, including where it applies.

        Assembled here rather than field by field at the call site: the place
        that knows what a stop policy is also knows what relaxes it.
        """
        return cls(
            grace_seconds=execution.cancellation_grace_seconds,
            poll_seconds=execution.cancellation_poll_seconds,
            refuse_survivors=not os.environ.get(execution.survivors_unenforced_when_set),
        )

    def __post_init__(self) -> None:
        if self.grace_seconds <= 0 or self.poll_seconds <= 0:
            raise ValueError("process stop policy durations must be positive")


def run(
    argv: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    capture: bool,
    policy: StopPolicy,
) -> subprocess.CompletedProcess[str]:
    """Run a foreground command and keep its whole process group owned."""
    process = subprocess.Popen(
        list(argv),
        cwd=str(cwd),
        env=dict(env),
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        start_new_session=True,
    )
    owned: OwnedTree = {}
    try:
        if capture:
            stdout, stderr = _communicate(process, policy, owned)
        else:
            _wait(process, policy, owned)
            stdout = stderr = None
        _refuse_descendants(process, policy, owned)
    except BaseException:
        try:
            _terminate(process, policy, owned)
        finally:
            _close_pipes(process)
        raise
    return subprocess.CompletedProcess(list(argv), process.returncode, stdout, stderr)


def tee(
    argv: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    write: Callable[[str], None],
    policy: StopPolicy,
) -> int:
    """Run a foreground command while filing each available output chunk."""
    process = subprocess.Popen(
        list(argv),
        cwd=str(cwd),
        env=dict(env),
        text=False,
        bufsize=0,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    owned: OwnedTree = {}
    assert process.stdout is not None
    try:
        with selectors.DefaultSelector() as ready:
            ready.register(process.stdout, selectors.EVENT_READ)
            while ready.get_map():
                _remember_descendants(process.pid, owned)
                events = ready.select(timeout=policy.poll_seconds)
                if not events:
                    if process.poll() is not None:
                        _refuse_descendants(process, policy, owned)
                    cancellation.check(f"foreground process {process.pid}")
                    continue
                for key, _mask in events:
                    chunk = os.read(key.fd, 65536)
                    if chunk:
                        write(chunk.decode("utf-8", errors="replace"))
                    else:
                        ready.unregister(key.fileobj)
        _wait(process, policy, owned)
        _refuse_descendants(process, policy, owned)
    except BaseException:
        _terminate(process, policy, owned)
        raise
    finally:
        process.stdout.close()
    return process.returncode


def _wait(
    process: ForegroundProcess,
    policy: StopPolicy,
    owned: OwnedTree,
) -> None:
    while True:
        _remember_descendants(process.pid, owned)
        try:
            process.wait(timeout=policy.poll_seconds)
            return
        except subprocess.TimeoutExpired:
            cancellation.check(f"foreground process {process.pid}")


def _communicate(
    process: subprocess.Popen[str], policy: StopPolicy, owned: OwnedTree
) -> tuple[str | None, str | None]:
    while True:
        _remember_descendants(process.pid, owned)
        try:
            return process.communicate(timeout=policy.poll_seconds)
        except subprocess.TimeoutExpired:
            if process.poll() is not None:
                _refuse_descendants(process, policy, owned)
            cancellation.check(f"foreground process {process.pid}")


def _refuse_descendants(process: ForegroundProcess, policy: StopPolicy, owned: OwnedTree) -> None:
    """A foreground leader may not turn its children into hidden daemons."""
    _remember_descendants(process.pid, owned)
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
    # Named before they are killed: `_terminate` is what makes them unnameable.
    # This fired in a release lane saying only that *something* survived, while
    # holding the process objects the whole time.
    surviving = _surviving(descendants)
    _terminate(process, policy, owned)
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
    """What is still alive, identified well enough to go looking for it."""
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


def _terminate(
    process: ForegroundProcess, policy: StopPolicy, owned: OwnedTree | None = None
) -> None:
    tracked = {} if owned is None else owned
    _remember_descendants(process.pid, tracked)
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
        # A reaped leader's unsignalable or reused numeric group may no longer be ours.
        return False
    return True


def _signal_group(group: int, sent: signal.Signals) -> None:
    with suppress(ProcessLookupError, PermissionError):
        os.killpg(group, sent)


def _remember_descendants(pid: int, owned: OwnedTree) -> None:
    try:
        descendants = SystemProcess(pid).children(recursive=True)
    except NoSuchProcess:
        return
    for process in descendants:
        owned.setdefault(process.pid, process)


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


def _close_pipes(process: ForegroundProcess) -> None:
    for pipe in (process.stdout, process.stderr):
        if pipe is not None:
            pipe.close()

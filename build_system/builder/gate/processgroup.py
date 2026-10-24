"""One foreground command, its descendants, and how they stop together.

Foreground work belongs to the gate for exactly as long as its action. A command
that needs to outlive that action uses ``Runner.launch``; everything else gets a
fresh process group so cancellation never guesses at names or touches other work.
"""

from __future__ import annotations

import os
import selectors
import subprocess
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Protocol

from . import cancellation, processstop
from .commandtimeout import Deadline
from .processstop import ForegroundProcess, OwnedTree, StopPolicy, terminate


class ForegroundObserver(Protocol):
    """Optional resource observer; the process owner keeps all lifecycle authority."""

    def started(self, process: ForegroundProcess) -> None: ...
    def before_wait(self, process: ForegroundProcess) -> None: ...
    def retired(self, process: ForegroundProcess) -> None: ...
    def close(self) -> None: ...


def run(
    argv: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    capture: bool,
    policy: StopPolicy,
    timeout_seconds: float | None = None,
) -> subprocess.CompletedProcess[str]:
    """Run a foreground command and keep its whole process group owned."""
    deadline = Deadline.start(timeout_seconds)
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
            stdout, stderr = _communicate(process, policy, owned, deadline)
        else:
            _wait(process, policy, owned, deadline)
            stdout = stderr = None
        processstop.refuse_descendants(process, policy, owned)
    except BaseException:
        try:
            terminate(process, policy, owned)
        finally:
            _close_pipes(process)
        raise
    return subprocess.CompletedProcess(list(argv), process.returncode, stdout, stderr)


def tee(
    argv: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    write: Callable[[str], None] | None = None,
    write_bytes: Callable[[bytes], None] | None = None,
    stderr: int | None = subprocess.STDOUT,
    executable: Path | None = None,
    policy: StopPolicy,
    timeout_seconds: float | None = None,
    observer: ForegroundObserver | None = None,
) -> int:
    """File output through exactly one callback, with unchanged text semantics.

    Protocol owners select original bytes and inherited stderr to keep diagnostic
    output separate. Their callback enforces its byte budget during this drain.
    Callback failure uses the same exact-owned tree teardown as timeouts.
    """
    if (write is None) == (write_bytes is None):
        raise ValueError("tee requires exactly one text or byte callback")
    if stderr not in (None, subprocess.STDOUT):
        raise ValueError("tee diagnostics must be inherited or merged, never an undrained pipe")
    if observer is not None and not policy.refuse_survivors:
        raise ValueError("observed workloads require strict descendant settlement")
    deadline = Deadline.start(timeout_seconds)
    process = subprocess.Popen(
        list(argv),
        cwd=str(cwd),
        env=dict(env),
        text=False,
        bufsize=0,
        stdout=subprocess.PIPE,
        stderr=stderr,
        executable=executable,
        start_new_session=True,
    )
    owned: OwnedTree = {}
    assert process.stdout is not None
    try:
        if observer is not None:
            observer.started(process)
        with selectors.DefaultSelector() as ready:
            ready.register(process.stdout, selectors.EVENT_READ)
            while ready.get_map():
                if observer is not None and process.returncode is None:
                    observer.before_wait(process)
                processstop.remember_descendants(process.pid, owned)
                events = ready.select(timeout=deadline.poll_seconds(policy.poll_seconds))
                if not events:
                    if process.poll() is not None:
                        processstop.refuse_descendants(process, policy, owned)
                    cancellation.check(f"foreground process {process.pid}")
                    deadline.check()
                    continue
                for key, _mask in events:
                    chunk = os.read(key.fd, 65536)
                    if chunk:
                        if write_bytes is not None:
                            write_bytes(chunk)
                        else:
                            assert write is not None
                            write(chunk.decode("utf-8", errors="replace"))
                    else:
                        ready.unregister(key.fileobj)
        _wait(process, policy, owned, deadline, observer)
        processstop.refuse_descendants(process, policy, owned)
        if observer is not None:
            observer.retired(process)
    except BaseException:
        terminate(process, policy, owned)
        raise
    finally:
        try:
            if observer is not None:
                observer.close()
        finally:
            process.stdout.close()
    return process.returncode


def _wait(
    process: ForegroundProcess,
    policy: StopPolicy,
    owned: OwnedTree,
    deadline: Deadline | None = None,
    observer: ForegroundObserver | None = None,
) -> None:
    deadline = deadline or Deadline.start(None)
    while True:
        if observer is not None and process.returncode is None:
            observer.before_wait(process)
        processstop.remember_descendants(process.pid, owned)
        try:
            process.wait(timeout=deadline.poll_seconds(policy.poll_seconds))
            return
        except subprocess.TimeoutExpired:
            cancellation.check(f"foreground process {process.pid}")
            deadline.check()


def _communicate(
    process: subprocess.Popen[str],
    policy: StopPolicy,
    owned: OwnedTree,
    deadline: Deadline | None = None,
) -> tuple[str | None, str | None]:
    deadline = deadline or Deadline.start(None)
    while True:
        processstop.remember_descendants(process.pid, owned)
        try:
            return process.communicate(timeout=deadline.poll_seconds(policy.poll_seconds))
        except subprocess.TimeoutExpired:
            if process.poll() is not None:
                processstop.refuse_descendants(process, policy, owned)
            cancellation.check(f"foreground process {process.pid}")
            deadline.check()


def _close_pipes(process: ForegroundProcess) -> None:
    for pipe in (process.stdout, process.stderr):
        if pipe is not None:
            pipe.close()

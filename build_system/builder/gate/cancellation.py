"""Stopping a run that is already going, without stopping it mid-write.

Ctrl-C used to mean "wait". `planrunner` owned its `ThreadPoolExecutor` through
a `with` block, and that context manager's exit joins every running future, so
an interrupt fifty milliseconds into a 750ms action returned after 756ms. A
real copy, hash or image assembly runs far longer than that, and signals only
reach Python on the main thread -- so the operator pressed Ctrl-C and watched
nothing happen.

Returning immediately would be worse. The machine lock, the workspace and the
service are released on the way out, and releasing them under a worker still
writing into them turns an interrupt into corruption. What was missing is not a
faster exit but a way to *ask*: a flag the long primitives read at points where
stopping is safe.

A `ContextVar`, like the plan seal and the current step: a primitive deep
inside a copy can ask whether the run is still wanted without every function
between here and there carrying an argument for it.

Note that `ThreadPoolExecutor` does *not* copy the submitting context into its
workers -- `asyncio.to_thread` does, `concurrent.futures` does not -- so a
worker adopts the switch explicitly through `observing`. Assuming otherwise
gives you a flag that is set, read, and always false.
"""

from __future__ import annotations

import signal
import threading
from collections.abc import Iterator
from contextlib import contextmanager
from contextvars import ContextVar
from types import FrameType

from .errors import GateError

_CANCELLED: ContextVar[threading.Event | None] = ContextVar("capsem_gate_cancelled", default=None)


class Cancelled(GateError):
    """Raised inside a worker that noticed the run was abandoned.

    A `GateError` so the normal failure path reports it by step name rather
    than unwinding as something nobody classified -- but the run's *status*
    comes from the interrupt on the main thread, not from this. An interrupted
    gate is not a gate that failed its checks.
    """


class Terminated(BaseException):
    """A supervisor asked the gate to stop, while preserving unwinding."""

    def __init__(self, received: signal.Signals) -> None:
        self.received = received
        super().__init__(f"terminated by {received.name}")

    @property
    def exit_status(self) -> int:
        return 128 + int(self.received)


@contextmanager
def unwind_sigterm() -> Iterator[None]:
    """Translate SIGTERM into an exception so held state and journals close.

    SIGINT already becomes ``KeyboardInterrupt``. Process supervisors and
    wrappers use SIGTERM instead, whose Python default exits immediately and
    skips every context-manager cleanup. SIGKILL remains the deliberate
    uncatchable escape hatch.
    """
    previous = signal.getsignal(signal.SIGTERM)

    def terminate(received: int, _frame: FrameType | None) -> None:
        raise Terminated(signal.Signals(received))

    signal.signal(signal.SIGTERM, terminate)
    try:
        yield
    finally:
        signal.signal(signal.SIGTERM, previous)


@contextmanager
def cancellable() -> Iterator[threading.Event]:
    """Arm cancellation for the duration, and hand back its switch."""
    flag = threading.Event()
    token = _CANCELLED.set(flag)
    try:
        yield flag
    finally:
        _CANCELLED.reset(token)


@contextmanager
def observing(flag: threading.Event) -> Iterator[None]:
    """Adopt an existing switch in this thread.

    What a pool worker calls. The switch belongs to the run; each worker has
    its own context and has to be handed it.
    """
    token = _CANCELLED.set(flag)
    try:
        yield
    finally:
        _CANCELLED.reset(token)


def stopped() -> bool:
    """Whether the run has been abandoned. Cheap enough for a tight loop."""
    flag = _CANCELLED.get()
    return flag is not None and flag.is_set()


def check(what: str) -> None:
    """Give up here, if here is a safe place to give up.

    Call it at boundaries a partially-done unit can be abandoned from -- between
    files, between chunks -- never in the middle of writing one.
    """
    if stopped():
        raise Cancelled(f"{what} stopped: the run was interrupted")

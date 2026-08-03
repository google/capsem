"""Executing a plan: the scheduler, the locks, and what a failure means.

Split from `plan` for the reason the whole package is split -- one
responsibility each. `plan` holds the graph and the edges, `planchecks` says
what makes one well-formed, `planreport` explains it, and this runs it.

The scheduler streams rather than working in waves. `TopologicalSorter`'s
`get_ready`/`done` protocol hands back the next runnable steps as earlier ones
finish, so a long step never holds up work that became ready behind it. Waves
exist for reading, not for running.

Contention is the exception the graph cannot express: two steps may be
genuinely independent and still unable to share the machine, because they both
launch VMs or both drive the one service-scoped snapshot lock. Each exclusive
gets a lock, and a step takes its own in sorted order -- sorted so that two
steps claiming the same pair in opposite orders is unrepresentable rather than
merely unlikely.
"""

from __future__ import annotations

import threading
import time
from collections.abc import Iterator
from concurrent.futures import FIRST_COMPLETED, Future, ThreadPoolExecutor, wait
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from operator import attrgetter
from typing import TYPE_CHECKING

from .cancellation import cancellable, check, observing
from .context import Context
from .errors import GateError
from .execution import Step

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .plan import Plan

#: What a step's outcome was. `skipped` is deliberately distinct from `failed`:
#: a step that never ran because its dependency broke did not fail, and a
#: report that conflates the two hides the blast radius of the real failure.
OK, FAILED, SKIPPED = "ok", "failed", "skipped"


@dataclass
class Outcome:
    """What one step did, and how long it took."""

    label: str
    status: str
    duration: float = 0.0
    error: BaseException | None = None


def execute(plan: Plan, context: Context) -> dict[str, Outcome]:
    """Run every step the graph allows, and report what each one did."""
    sorter = plan.sorter()
    context.journal.shape(plan.labels, plan.edges)
    outcomes: dict[str, Outcome] = {}
    locks = {resource.name: _SharedLock() for step in plan.steps for resource in step.contends}
    broken: set[str] = set()

    pool = ThreadPoolExecutor(max_workers=max(len(plan.steps), 1))
    running: dict[Future[float], Step] = {}
    with cancellable() as abandoned:
        try:
            while sorter.is_active():
                for label in sorter.get_ready():
                    step = plan.step_named(label)
                    if plan.after_of(label) & broken:
                        # Its inputs were never produced. Running it would
                        # report a second failure that is really the first one.
                        outcomes[label] = Outcome(label, SKIPPED)
                        context.journal.skipped(label)
                        broken.add(label)
                        sorter.done(label)
                        continue
                    running[pool.submit(_guarded, step, context, locks, abandoned)] = step

                if not running:
                    continue
                finished = next(iter(_completed(running)))
                step = running.pop(finished)
                _record(step, finished, outcomes, broken)
                sorter.done(step.label)
        except BaseException:
            _abandon(pool, running, locks, abandoned, context)
            raise
        else:
            pool.shutdown(wait=True)

    return outcomes


#: How long an interrupted run waits for its workers before saying who is still
#: going. Long enough for a primitive to reach its next boundary, short enough
#: that Ctrl-C means something.
GRACE_SECONDS = 10.0


def _abandon(
    pool: ThreadPoolExecutor,
    running: dict[Future[float], Step],
    locks: dict[str, _SharedLock],
    abandoned: threading.Event,
    context: Context,
) -> None:
    """Stop, in the order that makes stopping safe.

    An interrupt used to mean "wait": the executor was held through a `with`,
    whose exit joins every running future, so Ctrl-C fifty milliseconds into a
    long action returned when that action finished. Returning immediately would
    be worse -- the machine lock, the workspace and the service are released on
    the way out of `execute`, and releasing them under a worker still writing is
    how an interrupt becomes corruption.

    So: ask, then wake anything asleep, then wait a bounded while, then say who
    did not stop. The waiting is what makes the release below it safe; the bound
    is what stops it being indefinite.
    """
    abandoned.set()
    # Anything not started must not start. Anything asleep on a resource has to
    # wake to notice the flag, or it waits for a holder that is also stopping.
    for future in running:
        future.cancel()
    for lock in locks.values():
        lock.wake()

    pool.shutdown(wait=False, cancel_futures=True)
    _done, pending = wait(running, timeout=GRACE_SECONDS)
    if pending:
        stubborn = sorted(running[future].label for future in pending)
        context.journal.note(
            f"interrupted; still running after {GRACE_SECONDS:.0f}s: {', '.join(stubborn)}"
        )


class _SharedLock:
    """One resource, two kinds of holder.

    Shared holders admit each other; an exclusive holder admits nobody. A
    readers-writer lock, because that is the shape the asset lanes always had:
    two architectures that must overlap, against every other Docker step that
    must not run beside them.

    Writers are given the door as soon as one is waiting, so a steady stream
    of lanes cannot starve the phase that follows them.
    """

    def __init__(self) -> None:
        self._state = threading.Condition()
        self._readers = 0
        self._writer = False
        self._writers_waiting = 0

    @contextmanager
    def held(self, *, shared: bool) -> Iterator[None]:
        self._acquire(shared=shared)
        try:
            yield
        finally:
            self._release(shared=shared)

    def _acquire(self, *, shared: bool) -> None:
        with self._state:
            if shared:
                while self._writer or self._writers_waiting:
                    self._state.wait()
                    check("waiting for a shared resource")
                self._readers += 1
                return
            self._writers_waiting += 1
            try:
                while self._writer or self._readers:
                    self._state.wait()
                    check("waiting for an exclusive resource")
            finally:
                self._writers_waiting -= 1
            self._writer = True

    def _release(self, *, shared: bool) -> None:
        with self._state:
            if shared:
                self._readers -= 1
            else:
                self._writer = False
            self._state.notify_all()

    def wake(self) -> None:
        """Wake every waiter without granting anything.

        For an interrupted run: a step asleep here is waiting on a holder that
        is also stopping, so nobody would notify it. It wakes, re-checks its
        condition, and its own cancellation check ends it.
        """
        with self._state:
            self._state.notify_all()


def _guarded(
    step: Step,
    context: Context,
    locks: dict[str, _SharedLock],
    abandoned: threading.Event,
) -> float:
    """Hold what the step contends for, run it, and report how long.

    `abandoned` is passed rather than inherited: a pool worker starts with a
    fresh context, so the run's cancellation switch has to be handed to it.
    """
    started = time.monotonic()
    with observing(abandoned), ExitStack() as stack:
        for resource in sorted(step.contends, key=attrgetter("name")):
            stack.enter_context(locks[resource.name].held(shared=resource.shared))
        with context.journal.step(step):
            step.run(context)
    return time.monotonic() - started


def _record(
    step: Step,
    future: Future[float],
    outcomes: dict[str, Outcome],
    broken: set[str],
) -> None:
    error = future.exception()
    if error is None:
        outcomes[step.label] = Outcome(step.label, OK, future.result())
        return
    if not isinstance(error, Exception):
        # An interrupt is not a step that failed, and recording it as one would
        # turn Ctrl-C into a gate result. This is the hazard the shell trap had
        # in another form, where `$?` inside EXIT read 0 on abort and reported
        # an interrupted run as a pass.
        raise error
    outcomes[step.label] = Outcome(step.label, FAILED, 0.0, error)
    broken.add(step.label)


def raise_for_failures(name: str, outcomes: dict[str, Outcome]) -> None:
    """Report every failure by name, and what never ran because of them.

    Separate from `execute` so the plan can record the outcomes *before* this
    raises. A failed run is exactly the run whose timings and critical path
    somebody wants, and they were empty at the only moment they mattered.
    """
    failed = [o for o in outcomes.values() if o.status == FAILED]
    if not failed:
        return
    skipped = sorted(o.label for o in outcomes.values() if o.status == SKIPPED)
    detail = "; ".join(f"{o.label}: {o.error}" for o in sorted(failed, key=attrgetter("label")))
    message = f"{name} failed -- {detail}"
    if skipped:
        message += f" (skipped, never ran: {', '.join(skipped)})"
    raise GateError(message)


def _completed(running: dict[Future[float], Step]) -> list[Future[float]]:
    """Block until at least one future is done, then return those that are.

    A plain `as_completed` would need rebuilding every time a wave adds work;
    waiting on the current set and returning is simpler, and lets the loop pick
    up newly ready steps immediately.
    """
    done, _pending = wait(running, return_when=FIRST_COMPLETED)
    return list(done)

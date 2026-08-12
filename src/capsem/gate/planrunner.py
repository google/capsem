"""Executing a plan: the scheduler, the claims, and what a failure means.

Split from `plan` for the reason the whole package is split -- one
responsibility each. `plan` holds the graph and the edges, `planchecks` says
what makes one well-formed, `planreport` explains it, and this runs it.

The scheduler streams rather than working in waves. `TopologicalSorter`'s
`get_ready`/`done` protocol hands back the next runnable steps as earlier ones
finish, so a long step never holds up work that became ready behind it. Waves
exist for reading, not for running.

Contention is the exception the graph cannot express: two steps may be
genuinely independent and still unable to share the machine, because they both
launch VMs or both drive the one service-scoped snapshot lock. Those claims are
reserved *here*, before a step is submitted. They were taken inside the worker,
which meant a worker could be occupied purely by waiting -- so the pool had to
be as large as the plan for the one step that could actually run to have
somewhere to go, and a step's recorded duration was its queue time plus its
work with no way to tell them apart.
"""

from __future__ import annotations

import threading
import time
from concurrent.futures import FIRST_COMPLETED, Future, ThreadPoolExecutor, wait
from contextlib import ExitStack
from dataclasses import dataclass, field
from operator import attrgetter
from typing import TYPE_CHECKING

from .cancellation import cancellable, observing
from .contention import Claims
from .context import Context
from .errors import GateError
from .execution import Step

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .plan import Plan

#: What a step's outcome was. `skipped` is deliberately distinct from `failed`:
#: a step that never ran because its dependency broke did not fail, and a
#: report that conflates the two hides the blast radius of the real failure.
from .runlogschema import CARRIED, FAILED, OK, SKIPPED


@dataclass
class Outcome:
    """What one step did, and where its latency went.

    Three numbers rather than one. A step that took twenty minutes because it
    queued nineteen of them behind Docker looked exactly like a step doing
    twenty minutes of work, and those have different fixes.
    """

    label: str
    status: str
    duration: float = 0.0
    error: BaseException | None = None

    dependency_wait: float = 0.0
    """From the start of the run until every step it waits on had finished."""

    resource_wait: float = 0.0
    """From dependency-ready until its claims were free and it was submitted."""

    execution: float = 0.0
    """Its own work, holding everything it contends for."""


@dataclass
class _Pending:
    """A dependency-ready step, and when it became one."""

    step: Step
    ready_at: float
    submitted_at: float = 0.0
    started: float = field(default=0.0)


def execute(plan: Plan, context: Context, *, max_parallel: int | None = None) -> dict[str, Outcome]:
    """Run every step the graph allows, and report what each one did."""
    sorter = plan.sorter()
    context.journal.shape(plan.labels, plan.edges)
    outcomes: dict[str, Outcome] = {}
    broken: set[str] = set()
    claims = Claims()
    limit = max_parallel or context.config.execution.max_parallel_steps

    began = time.monotonic()
    pool = ThreadPoolExecutor(max_workers=max(limit, 1))
    running: dict[Future[float], _Pending] = {}
    ready: list[_Pending] = []

    with cancellable() as abandoned:
        try:
            while sorter.is_active():
                for label in sorter.get_ready():
                    step = plan.step_named(label)
                    if label in context.carried:
                        # Proved by the run being resumed, and its outputs are
                        # still in the prefix this one reuses. Not `broken`:
                        # everything downstream may proceed.
                        outcomes[label] = Outcome(label, CARRIED)
                        context.journal.carried(label)
                        sorter.done(label)
                        continue
                    if plan.after_of(label) & broken:
                        # Its inputs were never produced. Running it would
                        # report a second failure that is really the first one.
                        outcomes[label] = Outcome(label, SKIPPED)
                        context.journal.skipped(label)
                        broken.add(label)
                        sorter.done(label)
                        continue
                    ready.append(_Pending(step, time.monotonic()))

                _start_what_fits(pool, ready, running, claims, limit, context, abandoned)

                if not running:
                    continue
                finished = next(iter(_completed(running)))
                pending = running.pop(finished)
                claims.release(pending.step)
                _record(pending, finished, outcomes, broken, began, context)
                sorter.done(pending.step.label)
        except BaseException:
            _abandon(pool, running, abandoned, context)
            raise
        else:
            pool.shutdown(wait=True)

    return outcomes


def _start_what_fits(
    pool: ThreadPoolExecutor,
    ready: list[_Pending],
    running: dict[Future[float], _Pending],
    claims: Claims,
    limit: int,
    context: Context,
    abandoned: threading.Event,
) -> None:
    """Submit every ready step whose claims are free, up to the bound.

    Reserved before submission, so a worker only ever holds what it already
    owns. Deadlock is not reachable: with nothing running, no claim is held,
    so the first ready step is always compatible.
    """
    for pending in list(ready):
        if len(running) >= limit:
            return
        if not claims.compatible(pending.step):
            continue
        claims.reserve(pending.step)
        ready.remove(pending)
        pending.submitted_at = time.monotonic()
        running[pool.submit(_guarded, pending, context, abandoned)] = pending


def _abandon(
    pool: ThreadPoolExecutor,
    running: dict[Future[float], _Pending],
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

    So: ask, then wait a bounded while, then say who did not stop. Nothing has
    to be woken any more; a step waiting for a resource was never submitted, so
    the only threads to reach are the ones genuinely working.
    """
    abandoned.set()
    for future in running:
        future.cancel()

    pool.shutdown(wait=False, cancel_futures=True)
    grace = context.config.execution.cancellation_grace_seconds
    _done, pending = wait(running, timeout=grace)
    if pending:
        stubborn = sorted(running[future].step.label for future in pending)
        context.journal.note(f"interrupted; still running after {grace:g}s: {', '.join(stubborn)}")


def _guarded(pending: _Pending, context: Context, abandoned: threading.Event) -> float:
    """Run the step, and report how long its own work took.

    Its claims are already reserved, so there is nothing to acquire here. What
    remains of the old `ExitStack` is the journal bracket.

    `abandoned` is passed rather than inherited: a pool worker starts with a
    fresh context, so the run's cancellation switch has to be handed to it.
    """
    pending.started = time.monotonic()
    with observing(abandoned), ExitStack() as stack:
        stack.enter_context(context.journal.step(pending.step))
        # Whoever is in flight owns whatever the disk does next. Attribution
        # has to come from the scheduler: it is the only thing that knows.
        if context.watch is not None:
            context.watch.entered(pending.step.label)
            stack.callback(context.watch.left, pending.step.label)
        pending.step.run(context)
    return time.monotonic() - pending.started


def _record(
    pending: _Pending,
    future: Future[float],
    outcomes: dict[str, Outcome],
    broken: set[str],
    began: float,
    context: Context,
) -> None:
    label = pending.step.label
    # Named rather than splatted: a `**dict[str, float]` is a dict as far as a
    # type checker is concerned, so it has to assume it might land on `error`.
    dependency_wait = pending.ready_at - began
    resource_wait = pending.submitted_at - pending.ready_at
    error = future.exception()
    if error is None:
        outcomes[label] = Outcome(
            label,
            OK,
            duration=time.monotonic() - pending.ready_at,
            dependency_wait=dependency_wait,
            resource_wait=resource_wait,
            execution=future.result(),
        )
        context.journal.waited(
            label,
            dependency_ms=dependency_wait * 1000,
            resource_ms=resource_wait * 1000,
            execution_ms=future.result() * 1000,
        )
        return
    if not isinstance(error, Exception):
        # An interrupt is not a step that failed, and recording it as one would
        # turn Ctrl-C into a gate result. This is the hazard the shell trap had
        # in another form, where `$?` inside EXIT read 0 on abort and reported
        # an interrupted run as a pass.
        raise error
    outcomes[label] = Outcome(
        label,
        FAILED,
        error=error,
        dependency_wait=dependency_wait,
        resource_wait=resource_wait,
    )
    broken.add(label)


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


def _completed(running: dict[Future[float], _Pending]) -> list[Future[float]]:
    """Block until at least one future is done, then return those that are.

    A plain `as_completed` would need rebuilding every time a wave adds work;
    waiting on the current set and returning is simpler, and lets the loop pick
    up newly ready steps immediately.
    """
    done, _pending = wait(running, return_when=FIRST_COMPLETED)
    return list(done)

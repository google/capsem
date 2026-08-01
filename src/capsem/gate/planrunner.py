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
from concurrent.futures import FIRST_COMPLETED, Future, ThreadPoolExecutor, wait
from contextlib import ExitStack
from dataclasses import dataclass
from operator import attrgetter
from typing import TYPE_CHECKING

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
    locks = {
        resource.name: threading.Lock()
        for step in plan.steps
        for resource in step.contends
    }
    broken: set[str] = set()

    with ThreadPoolExecutor(max_workers=max(len(plan.steps), 1)) as pool:
        running: dict[Future[float], Step] = {}
        while sorter.is_active():
            for label in sorter.get_ready():
                step = plan.step_named(label)
                if plan.after_of(label) & broken:
                    # Its inputs were never produced. Running it would report a
                    # second failure that is really the first one.
                    outcomes[label] = Outcome(label, SKIPPED)
                    context.journal.skipped(label)
                    broken.add(label)
                    sorter.done(label)
                    continue
                running[pool.submit(_guarded, step, context, locks)] = step

            if not running:
                continue
            finished = next(iter(_completed(running)))
            step = running.pop(finished)
            _record(step, finished, outcomes, broken)
            sorter.done(step.label)

    _raise_for_failures(plan.name, outcomes)
    return outcomes


def _guarded(
    step: Step, context: Context, locks: dict[str, threading.Lock]
) -> float:
    """Hold what the step contends for, run it, and report how long."""
    started = time.monotonic()
    with ExitStack() as stack:
        for resource in sorted(step.contends, key=attrgetter("name")):
            stack.enter_context(locks[resource.name])
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


def _raise_for_failures(name: str, outcomes: dict[str, Outcome]) -> None:
    failed = [o for o in outcomes.values() if o.status == FAILED]
    if not failed:
        return
    skipped = sorted(o.label for o in outcomes.values() if o.status == SKIPPED)
    detail = "; ".join(
        f"{o.label}: {o.error}" for o in sorted(failed, key=attrgetter("label"))
    )
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

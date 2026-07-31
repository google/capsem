"""The steps of one command, and what must finish before what.

Order is declared, then derived. A step is registered with the steps it must
follow, `graphlib.TopologicalSorter` decides the sequence, and two properties
fall out that a hand-written list cannot have.

A cycle is a plan-time error naming the steps involved, rather than a hang or a
wrong order discovered forty minutes in. And whatever the sort makes
simultaneously ready is independent *by construction*, so concurrency stops
being a human judgement about which jobs are safe beside each other -- which is
what seven bare `&` in one recipe body were.

Contention is the exception the graph cannot express: two steps may be
genuinely independent and still unable to share the machine, because they both
launch VMs or both drive the one service-scoped snapshot lock. Each exclusive
gets a lock, and a step takes its own in sorted order -- sorted so that two
steps claiming the same pair in opposite orders is unrepresentable rather than
merely unlikely.

`graphlib` rather than a graph library: its `prepare`/`get_ready`/`done`
protocol is execution-oriented, streaming the next runnable steps as earlier
ones finish, so a long step never holds up work that became ready behind it. A
library would supply the sort we already have and nothing else we use.
"""

from __future__ import annotations

import threading
import time
from concurrent.futures import FIRST_COMPLETED, Future, ThreadPoolExecutor, wait
from contextlib import ExitStack
from dataclasses import dataclass
from graphlib import CycleError, TopologicalSorter
from operator import attrgetter

from .context import Context
from .errors import GateError
from .execution import Step

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


class Plan:
    """A command's steps, the edges between them, and how they are run."""

    def __init__(self, name: str) -> None:
        self.name = name
        self._steps: list[Step] = []
        self._after: dict[str, set[str]] = {}
        self._by_label: dict[str, Step] = {}
        self._outcomes: dict[str, Outcome] = {}

    # -- building ----------------------------------------------------------

    def add(self, step: Step, *, after: tuple[Step, ...] = ()) -> Step:
        """Register a step, and return it so it can be named as a dependency."""
        if step.label in self._by_label:
            raise GateError(
                f"the {self.name} plan already has a step called {step.label!r}; "
                "two steps with one name cannot be told apart in the log"
            )
        self._steps.append(step)
        self._by_label[step.label] = step
        self._after[step.label] = set()
        for earlier in after:
            self.edge(before=earlier, after=step)
        return step

    def edge(self, *, before: Step, after: Step) -> None:
        """Order two registered steps.

        Both must already be registered: an edge naming a step this plan does
        not have would be silently dropped, and the order silently wrong.
        """
        for step in (before, after):
            if self._by_label.get(step.label) is not step:
                raise GateError(
                    f"{step.label!r} is not part of the {self.name} plan; "
                    "add it before making it a dependency"
                )
        self._after[after.label].add(before.label)

    # -- inspecting --------------------------------------------------------

    def order(self) -> list[tuple[Step, ...]]:
        """The steps in topological waves, earliest first.

        Waves are for reading -- the runner streams rather than working in
        batches -- but they are how a person understands the shape.
        """
        sorter = self._sorter()
        waves: list[tuple[Step, ...]] = []
        while sorter.is_active():
            ready = sorter.get_ready()
            waves.append(tuple(self._by_label[label] for label in ready))
            for label in ready:
                sorter.done(label)
        return waves

    def _sorter(self) -> TopologicalSorter[str]:
        sorter: TopologicalSorter[str] = TopologicalSorter(self._after)
        try:
            sorter.prepare()
        except CycleError as cycle:
            members = " -> ".join(str(part) for part in cycle.args[1])
            raise GateError(
                f"the {self.name} plan has a cycle, so no step can be first: "
                f"{members}"
            ) from None
        return sorter

    def describe(self) -> str:
        """The dry run: what would run, in what order, and what it invokes."""
        waves = self.order()
        actions = sum(len(step.actions) for step in self._steps)
        lines = [
            f"plan: {self.name} -- {len(self._steps)} steps, "
            f"{actions} actions, {len(waves)} waves",
            "",
        ]
        for position, wave in enumerate(waves, start=1):
            for offset, step in enumerate(sorted(wave, key=attrgetter("label"))):
                held = (
                    "  [" + ", ".join(sorted(e.name for e in step.contends)) + "]"
                    if step.contends
                    else ""
                )
                # The wave number once, on its first step: everything under it
                # runs at the same time, and repeating the number says the
                # opposite to anyone skimming.
                marker = f"{position:>3}" if offset == 0 else "   "
                lines.append(f"  {marker}  {step.label}{held}")
                lines += [f"          {rendering}" for rendering in step.render()]
        lines += ["", "nothing was executed (--dry-run)"]
        return "\n".join(lines)

    def mermaid(self) -> str:
        """The graph, for a bug report or the documentation site."""
        lines = ["graph TD"]
        for step in self._steps:
            lines.append(f"  {_node(step.label)}[{step.label}]")
        lines += [f"  {_node(before)} --> {_node(after)}" for before, after in self.edges]
        return "\n".join(lines)

    def critical_path(self) -> list[Step]:
        """The longest chain by measured duration.

        Not the slowest step: shortening that does nothing if it runs beside
        something longer. The critical path is what the run's duration is
        actually made of, and therefore the only thing worth shortening.
        """
        if not self._outcomes:
            raise GateError(f"the {self.name} plan has not run, so it has no timings")

        best: dict[str, tuple[float, list[str]]] = {}
        for wave in self.order():
            for step in wave:
                spent = self._outcomes[step.label].duration
                prior = max(
                    (best[earlier] for earlier in self._after[step.label]),
                    default=(0.0, []),
                )
                best[step.label] = (prior[0] + spent, [*prior[1], step.label])

        _total, path = max(best.values(), key=lambda entry: entry[0])
        return [self._by_label[label] for label in path]

    @property
    def labels(self) -> tuple[str, ...]:
        """Every step, in an order the graph allows -- as the log records it."""
        return tuple(step.label for wave in self.order() for step in wave)

    @property
    def edges(self) -> tuple[tuple[str, str], ...]:
        """`(before, after)` pairs, for the log and for the diagram."""
        return tuple(
            (before, after)
            for after, earlier in sorted(self._after.items())
            for before in sorted(earlier)
        )

    @property
    def outcomes(self) -> dict[str, Outcome]:
        return dict(self._outcomes)

    # -- running -----------------------------------------------------------

    def run(self, context: Context) -> None:
        """Execute, honouring the graph and what each step contends for."""
        sorter = self._sorter()
        context.journal.shape(self.labels, self.edges)
        self._outcomes = {}
        locks = {
            resource.name: threading.Lock()
            for step in self._steps
            for resource in step.contends
        }
        broken: set[str] = set()

        with ThreadPoolExecutor(max_workers=max(len(self._steps), 1)) as pool:
            running: dict[Future[float], Step] = {}
            while sorter.is_active():
                for label in sorter.get_ready():
                    step = self._by_label[label]
                    if self._after[label] & broken:
                        # Its inputs were never produced. Running it would
                        # report a second failure that is really the first one.
                        self._outcomes[label] = Outcome(label, SKIPPED)
                        broken.add(label)
                        sorter.done(label)
                        continue
                    running[pool.submit(self._guarded, step, context, locks)] = step

                if not running:
                    continue
                finished = next(iter(_completed(running)))
                step = running.pop(finished)
                self._record(step, finished, broken)
                sorter.done(step.label)

        self._raise_for_failures()

    def _guarded(
        self, step: Step, context: Context, locks: dict[str, threading.Lock]
    ) -> float:
        """Hold what the step contends for, run it, and report how long."""
        started = time.monotonic()
        with ExitStack() as stack:
            for resource in sorted(step.contends, key=attrgetter("name")):
                stack.enter_context(locks[resource.name])
            with context.journal.step(step):
                step.run(context)
        return time.monotonic() - started

    def _record(self, step: Step, future: Future[float], broken: set[str]) -> None:
        error = future.exception()
        if error is None:
            self._outcomes[step.label] = Outcome(step.label, OK, future.result())
            return
        if not isinstance(error, Exception):
            # An interrupt is not a step that failed, and recording it as one
            # would turn Ctrl-C into a gate result. This is the hazard the
            # shell trap had in another form, where `$?` inside EXIT read 0 on
            # abort and reported an interrupted run as a pass.
            raise error
        self._outcomes[step.label] = Outcome(step.label, FAILED, 0.0, error)
        broken.add(step.label)

    def _raise_for_failures(self) -> None:
        failed = [o for o in self._outcomes.values() if o.status == FAILED]
        if not failed:
            return
        skipped = sorted(o.label for o in self._outcomes.values() if o.status == SKIPPED)
        detail = "; ".join(f"{o.label}: {o.error}" for o in sorted(failed, key=attrgetter("label")))
        message = f"{self.name} failed -- {detail}"
        if skipped:
            message += f" (skipped, never ran: {', '.join(skipped)})"
        raise GateError(message)


def _completed(running: dict[Future[float], Step]) -> list[Future[float]]:
    """Block until at least one future is done, then return those that are.

    A plain `as_completed` would need rebuilding every time a wave adds work;
    waiting on the current set and returning is simpler and lets the loop pick
    up newly ready steps immediately.
    """
    done, _pending = wait(running, return_when=FIRST_COMPLETED)
    return list(done)


def _node(label: str) -> str:
    """A mermaid-safe identifier for a step label."""
    return "".join(character if character.isalnum() else "_" for character in label)

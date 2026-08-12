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

from dataclasses import replace
from graphlib import CycleError, TopologicalSorter

from . import planchecks, planreport, planrunner
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import Step
from .planrunner import FAILED, OK, SKIPPED, Outcome
from .timing import longest_chain

__all__ = ["FAILED", "OK", "SKIPPED", "Outcome", "Phase", "Plan"]


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

    def phase(self, prefix: str) -> Phase:
        """A view that namespaces everything a fragment adds.

        Composed into one plan, `test-static` and `test-functional` both want a
        step called `sign`, and both legitimately -- the binaries are signed
        after the coverage build and again before the VM suites. Namespacing
        makes them `static.sign` and `functional.sign`, which is also what the
        run log and the timing report then say.
        """
        return Phase(self, prefix)

    def shared(self, step: Step, *, after: tuple[Step, ...] = ()) -> Step:
        """Register groundwork that several fragments each need, once.

        Composed into one plan, `install-image` and `cross-compile` both want
        the Linux builder image. Adding it twice is a duplicate-label error;
        building it twice is waste; and having one of them silently skip it is
        an ordering bug waiting for the day they stop running in that order.
        A shared step is a diamond -- added by whoever asks first, depended on
        by everyone.

        A step whose label matches but whose actions differ is still the
        ordinary duplicate bug, and is refused: this must not become the place
        that hides.
        """
        existing = self._by_label.get(step.label)
        if existing is None:
            return self.add(step, after=after)
        existing_checks = [check.render() for check in existing.carry_checks]
        wanted_checks = [check.render() for check in step.carry_checks]
        if (
            existing.render() != step.render()
            or existing_checks != wanted_checks
            or existing.resume is not step.resume
        ):
            raise GateError(
                f"two different steps in the {self.name} plan are both called "
                f"{step.label!r}:\n  {existing.render()} / {existing_checks}\n  "
                f"{step.render()} / {wanted_checks}; resume="
                f"{existing.resume.value}/{step.resume.value}"
            )
        for earlier in after:
            self.edge(before=earlier, after=existing)
        return existing

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

    def validate(self, config: GateConfig) -> None:
        """Everything a plan must hold before a lock is taken.

        Labels are already unique -- `add` refuses a duplicate -- so what is
        left is the graph and the two claims a step makes about the world.
        The checks themselves live in `planchecks`; this is where they are
        required.
        """
        planchecks.validate(self, config)

    @property
    def steps(self) -> tuple[Step, ...]:
        """Every registered step, in declaration order.

        Declaration order, not graph order: the checks walk all of them and
        must not pay for a topological sort to do it.
        """
        return tuple(self._steps)

    def _sorter(self) -> TopologicalSorter[str]:
        sorter: TopologicalSorter[str] = TopologicalSorter(self._after)
        try:
            sorter.prepare()
        except CycleError as cycle:
            members = " -> ".join(str(part) for part in cycle.args[1])
            raise GateError(
                f"the {self.name} plan has a cycle, so no step can be first: {members}"
            ) from None
        return sorter

    def describe(self, *, carried: frozenset[str] = frozenset()) -> str:
        """The dry run: what would run, in what order, and what it invokes."""
        return planreport.describe(self, carried=carried)

    def mermaid(self) -> str:
        """The graph, for a bug report or the documentation site."""
        return planreport.mermaid(self)

    def critical_path(self) -> list[Step]:
        """The longest chain by measured duration.

        Not the slowest step: shortening that does nothing if it runs beside
        something longer. The walk itself lives in `timing`, which does it
        over recorded events -- a second copy here would be one more place for
        the two answers to disagree.
        """
        if not self._outcomes:
            raise GateError(f"the {self.name} plan has not run, so it has no timings")

        spent = {label: outcome.duration for label, outcome in self._outcomes.items()}
        path = longest_chain(list(self.labels), dict(self._after), spent)
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
        """Execute, honouring the graph and what each step contends for.

        Validated again here rather than trusting the caller: `execute` checks
        before taking the lock, so a bad plan costs nothing; and a plan reached
        by any other route still cannot run unchecked.
        """
        self.validate(context.config)
        # Assigned before anything raises: a failed run is exactly the run
        # whose timings and critical path somebody wants, and leaving the
        # outcomes unset made them empty at the only moment they mattered.
        self._outcomes = planrunner.execute(self, context)
        planrunner.raise_for_failures(self.name, self._outcomes)

    # -- what the runner and the checks need to walk this graph ------------

    def sorter(self) -> TopologicalSorter[str]:
        """A prepared sorter over this plan's edges."""
        return self._sorter()

    def step_named(self, label: str) -> Step:
        return self._by_label[label]

    def after_of(self, label: str) -> set[str]:
        """The labels that must finish before `label` may start."""
        return self._after[label]


class Phase:
    """One fragment's steps, added to a shared plan under one namespace.

    Composed into a single plan, `test-static` and `test-functional` both want
    a step called `sign`, and both legitimately -- the binaries are signed after
    the coverage build and again before the VM suites. Namespacing makes them
    `static.sign` and `functional.sign`, which is also what the run log and the
    timing report then say, so a slow step names the phase it belongs to.
    """

    def __init__(self, plan: Plan, prefix: str) -> None:
        self._plan = plan
        self._prefix = prefix

    def add(self, step: Step, *, after: tuple[Step, ...] = ()) -> Step:
        return self._plan.add(replace(step, label=self.label(step.label)), after=after)

    def shared(self, step: Step, *, after: tuple[Step, ...] = ()) -> Step:
        """Groundwork several phases need, kept out of any one namespace."""
        return self._plan.shared(step, after=after)

    def label(self, name: str) -> str:
        return f"{self._prefix}.{name}"

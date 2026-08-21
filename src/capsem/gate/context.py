"""What an action is handed, instead of what it happened to close over.

Before this, a unit of gate work was a closure over whatever its module had in
scope: one module reached for `self._config`, another rebuilt the config from
`runner.root`, a third took the path it needed as a constructor argument. Three
routes to one value, and no way to move a piece of work between commands
without dragging its module along.

An action receives a `Context` and reaches for nothing else. That is what makes
the same step reusable in a command that sequences it differently, and it is
what lets a test hand an action a recording runner and a list.
"""

from __future__ import annotations

from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import TYPE_CHECKING, Protocol

from .config import GateConfig
from .proc import Runner
from .runlogschema import OutputSpan

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from contextlib import AbstractContextManager

    from .actions import Action
    from .execution import Step


class StepObserver(Protocol):
    """What the scheduler tells the run's filesystem observer.

    A protocol rather than the concrete `Watch`, so nothing below the harness
    depends on how observation is implemented -- the same reason `Journal` is
    one.
    """

    def entered(self, label: str) -> None:
        """This step's thread is now running."""

    def left(self, label: str) -> None:
        """It is not."""

    def checkpoint(self) -> None:
        """Raise a pending asynchronous fault on this worker thread."""


class Journal(Protocol):
    """What a run is recorded into, as the rest of the package sees it.

    A protocol rather than the concrete run log, so nothing below the harness
    depends on how a run is stored and a test can pass a list. The plan marks
    boundaries; an action reports what it produced. Neither decides where any
    of it goes.
    """

    run_id: str

    def note(self, message: str) -> None:
        """Record something worth reading back, without failing anything."""

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        """Record bytes this run produced.

        So a run log can answer "which bytes did this gate build" without
        re-hashing an asset tree that may already have been reclaimed.
        """

    def shape(self, steps: tuple[str, ...], edges: tuple[tuple[str, str], ...]) -> None:
        """Record the graph, so a finished run can still be explained."""

    def exec(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        exit: int,
        duration_ms: float,
        output: OutputSpan | None = None,
    ) -> None:
        """Record one subprocess.

        Called by the runner rather than by whatever wanted the command, so
        that no call site can be the one that forgets. `env` is the delta the
        command added, never the ambient environment -- this record is attached
        to bug reports and a release machine's environment holds tokens.

        `output` points at the bytes this command wrote in its step's log, so
        one command's output can be read back out of a file every command in
        the step shares.
        """

    def launch(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        pid: int,
        duration_ms: float,
    ) -> None:
        """Record one detached process.

        Separate from `exec` because there is no exit status to wait for: what
        a launch can report is that it started, what it started, and as whom.
        A daemon nobody wrote down is a daemon nobody can account for when it
        outlives the run -- which is the case the orphan count exists for.
        """

    def step_output(self) -> Path | None:
        """Where the running step's command output belongs, if a step is running.

        Asked of the journal rather than passed by each call site: `log=` was a
        parameter roughly three lanes remembered, so every other command's
        output existed only in a terminal.
        """

    def waited(
        self, label: str, *, dependency_ms: float, resource_ms: float, execution_ms: float
    ) -> None:
        """Record where one step's latency went.

        From the coordinator, which is the only thing that can see it: a step
        knows how long its own work took and nothing about how long it queued
        for the resource somebody else was holding.
        """

    def carried(self, label: str) -> None:
        """Record a step a previous run proved, which `--resume` did not repeat.

        Its own event rather than `ok`, because a resumed run is not a clean
        proof of the whole graph and the log is the only place that survives to
        whoever reads the result.
        """

    def skipped(self, label: str) -> None:
        """Record a step that never ran because its dependency failed.

        Distinct from a failure: it did not fail, and conflating the two hides
        how far the real one reached. `RunLog` had this method and the plan
        never called it, so a run log showed the failure and no trace of the
        work it prevented.
        """

    def step(self, step: Step) -> AbstractContextManager[None]:
        """Bracket a step, recording what it was and how long it took."""

    def action(self, action: Action) -> AbstractContextManager[None]:
        """Bracket one primitive, so a slow line can name itself."""


class NullJournal:
    """Writes nothing.

    The default, so a plan or an action can be exercised -- in a test, or in a
    command asked only what it would do -- without a run behind it.
    """

    run_id = ""

    def note(self, message: str) -> None:
        """Discarded."""

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        """Discarded."""

    def shape(self, steps: tuple[str, ...], edges: tuple[tuple[str, str], ...]) -> None:
        """Discarded."""

    def exec(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        exit: int,
        duration_ms: float,
        output: OutputSpan | None = None,
    ) -> None:
        """Discarded."""

    def launch(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        pid: int,
        duration_ms: float,
    ) -> None:
        """Discarded."""

    def step_output(self) -> Path | None:
        """Nowhere: a run that is not being recorded has no step to file under."""
        return None

    def waited(
        self, label: str, *, dependency_ms: float, resource_ms: float, execution_ms: float
    ) -> None:
        """Discarded."""

    def carried(self, label: str) -> None:
        """Discarded."""

    def skipped(self, label: str) -> None:
        """Discarded."""

    @contextmanager
    def step(self, step: Step) -> Iterator[None]:
        yield

    @contextmanager
    def action(self, action: Action) -> Iterator[None]:
        yield


@dataclass(frozen=True)
class Context:
    """Everything an action needs, and nothing it does not."""

    runner: Runner
    config: GateConfig
    journal: Journal = field(default_factory=NullJournal)

    outside_runner: Runner | None = None
    """Authenticated pre-sandbox runner for explicitly networked edges."""

    watch: StepObserver | None = None
    """The run's filesystem observer, when one is running.

    Carried here so the scheduler can tell it which steps are in flight; an
    action never touches it.
    """

    observing: bool = False
    """This plan is being read, not run, so nothing may touch the machine.

    `tests/helpers/gate.py` reads back the argv a command would issue by
    *running* its plan against a recording runner. That stubs subprocesses and
    nothing else, so every filesystem action ran for real, against the real
    checkout, while a gate might be holding it. `RecordSourceState` overwrote
    the running gate's own record of what it was qualifying, and `source.verify`
    -- the last step of a forty-minute run -- reported a HEAD change on a tree
    nobody had touched.

    Declared here rather than checked action by action, because the next
    action to write a file will not remember either. It also makes an
    observation reach the *whole* plan: `Hash` fails on an artifact no build
    has produced, so observation used to stop at the first step that claimed
    an output and every later step went unseen.
    """

    carried: frozenset[str] = frozenset()
    """Steps a previous run already proved, which this one may skip.

    Only ever populated by `--resume`, and only for a run that is not
    qualifying a release. A carried step is recorded as `carried`, never `ok`:
    the evidence has to say which steps this process ran and which it took on
    trust, or a resumed run reads back as a clean one.
    """

    env: Mapping[str, str] = field(default_factory=dict)
    """Environment every action in this scope adds to its own.

    A workspace exports `CAPSEM_HOME` once, here, rather than every command
    inside it remembering to pass it -- which is how one of them stops
    remembering.
    """

    @property
    def root(self) -> Path:
        """The checkout the gate is running against."""
        return self.config.root

    def sandboxed(self) -> bool:
        """Whether a kernel sandbox is actually in force for this run.

        Read from the environment the command exported, which is the same
        value the sandbox itself was configured from. An action that declares
        it escapes has nothing to escape when this is false.
        """
        from .sandbox import OFF

        mode = self.env.get(self.config.environment.command_sandbox_mode, "")
        return bool(mode) and mode != OFF.value

    def path(self, relative: str) -> Path:
        return self.config.path(relative)

    def with_env(self, **extra: str) -> Context:
        """A child context adding environment, leaving this one untouched.

        Frozen and copied rather than mutated, so a step that adds an
        environment variable cannot change what a concurrent step sees.
        """
        return replace(self, env={**self.env, **extra})

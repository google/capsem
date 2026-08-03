"""How one event gets written down, and the boundaries that produce them.

Split from `runlog`, which owns what a *run* is -- allocating its directory,
protecting it while it is live, and closing it. This owns the other half: the
envelope every event carries, the file it is appended to, and the `step` and
`action` brackets that turn work into a record of work.

Every line is validated against a model on the way out. A log anything may
append to drifts into a shape nothing can read back, and the first person to
notice is the one who needed it.

Attribution is a `ContextVar`, not an attribute. The plan runs independent
steps concurrently, and one mutable string meant whichever step started last
owned every action, note, artifact and subprocess any of them emitted. Each
write was mutex-protected, which made the *lines* correct and the *attribution*
wrong -- and a run log that confidently blames the wrong step is worse than one
that says nothing.
"""

from __future__ import annotations

import json
import threading
import time
from contextlib import contextmanager
from contextvars import ContextVar
from pathlib import Path
from typing import TYPE_CHECKING

from .harnessschema import RunLogConfig
from .runlogschema import (
    ActionRun,
    Artifact,
    Exec,
    Launch,
    Note,
    Payload,
    PlanShape,
    StepEnd,
    StepStart,
    StepWaits,
)

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .actions import Action
    from .execution import Step

OK, FAILED, SKIPPED = "ok", "failed", "skipped"

#: Which step the current thread is inside. `step()` runs inside the worker
#: that owns it and sets this there, and each thread has its own context -- so
#: everything that step does, on that thread, is attributed to it and to
#: nothing else.
_CURRENT: ContextVar[str] = ContextVar("capsem_gate_current_step", default="")


class EventJournal:
    """Appends validated events to one file, and brackets the work.

    A base class rather than a mixin: `RunLog` *is* one of these, with a
    directory and a lifetime around it.
    """

    def __init__(self, events: Path, settings: RunLogConfig, *, run_id: str) -> None:
        self.settings = settings
        self.run_id = run_id
        self._events = events
        # Belt and braces. Each emit opens, writes one line, and closes, and an
        # `O_APPEND` write to a regular file is already atomic -- no test here
        # can be made to fail without this lock. It stays because the atomicity
        # is a property of the current write shape rather than of the
        # interface, and the day someone holds the handle open across several
        # writes this is what stops the log shredding itself.
        #
        # A mutex, not a scheduler: it creates no concurrency of its own.
        self._writing = threading.Lock()

    def emit(self, payload: Payload) -> None:
        """Append one validated event.

        The envelope is added here so it cannot be spelled differently by
        different callers.
        """
        line = json.dumps(
            {
                "schema": self.settings.event_schema,
                "ts": time.time(),
                "run_id": self.run_id,
                **payload.model_dump(),
            }
        )
        with self._writing, self._events.open("a", encoding="utf-8") as sink:
            sink.write(line + "\n")

    def shape(self, steps: tuple[str, ...], edges: tuple[tuple[str, str], ...]) -> None:
        """Record the graph, so this run can be explained after it is over."""
        self.emit(PlanShape(steps=steps, edges=edges))

    def note(self, message: str) -> None:
        self.emit(Note(step=_CURRENT.get(), message=message))

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        self.emit(Artifact(step=_CURRENT.get(), path=str(path), size=size, digest=digest))

    def exec(
        self, argv: tuple[str, ...], *, cwd: str, env: dict[str, str], exit: int, duration_ms: float
    ) -> None:
        self.emit(
            Exec(
                step=_CURRENT.get(),
                argv=argv,
                cwd=cwd,
                env=env,
                exit=exit,
                duration_ms=duration_ms,
            )
        )

    def launch(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        pid: int,
        duration_ms: float,
    ) -> None:
        self.emit(
            Launch(
                step=_CURRENT.get(),
                argv=argv,
                cwd=cwd,
                env=env,
                pid=pid,
                duration_ms=duration_ms,
            )
        )

    # -- boundaries --------------------------------------------------------

    @contextmanager
    def step(self, step: Step):
        """Record a step's boundaries and how long it took."""
        self.emit(
            StepStart(
                step=step.label,
                actions=len(step.actions),
                contends=tuple(sorted(e.name for e in step.contends)),
            )
        )
        started = time.monotonic()
        token = _CURRENT.set(step.label)
        try:
            yield
        except BaseException as error:
            self._end_step(step.label, FAILED, started, error)
            raise
        else:
            self._end_step(step.label, OK, started, None)
        finally:
            _CURRENT.reset(token)

    def _end_step(
        self, label: str, status: str, started: float, error: BaseException | None
    ) -> None:
        self.emit(
            StepEnd(
                step=label,
                status=status,
                duration_ms=(time.monotonic() - started) * 1000,
                error=None if error is None else str(error),
            )
        )

    def waited(
        self, label: str, *, dependency_ms: float, resource_ms: float, execution_ms: float
    ) -> None:
        """Where one step's latency went, as the coordinator observed it."""
        self.emit(
            StepWaits(
                step=label,
                dependency_ms=dependency_ms,
                resource_ms=resource_ms,
                execution_ms=execution_ms,
            )
        )

    def skipped(self, label: str) -> None:
        """A step that never ran because its dependency failed.

        Distinct from a failure: it did not fail, and conflating the two hides
        how far the real one reached.
        """
        self.emit(StepEnd(step=label, status=SKIPPED, duration_ms=0.0))

    @contextmanager
    def action(self, action: Action):
        """Record one primitive, so "the gate is slow" resolves to a line."""
        started = time.monotonic()
        try:
            yield
        except BaseException:
            self._end_action(action, FAILED, started)
            raise
        else:
            self._end_action(action, OK, started)

    def _end_action(self, action: Action, status: str, started: float) -> None:
        # An opaque action carries its own justification; a declared one has
        # nothing to justify, and an empty field is the honest answer.
        justification = getattr(action, "justification", None)
        self.emit(
            ActionRun(
                step=_CURRENT.get(),
                action=action.name,
                render=action.render(),
                duration_ms=(time.monotonic() - started) * 1000,
                status=status,
                opacity="" if justification is None else justification.kind.value,
                effects=() if justification is None else tuple(sorted(justification.effects)),
            )
        )

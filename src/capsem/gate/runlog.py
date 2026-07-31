"""One directory per run, so a failed gate is something you can attach.

Before this, diagnosing a gate failure meant having been present when it
happened. Which command ran with which arguments, what it exited with, how long
each phase took, which bytes came out -- all of it existed only as terminal
scrollback, and only for whoever was watching.

A run now writes an event stream, a log per step, and a summary. Every event is
validated against a model on the way out, because a log anything may append to
drifts into a shape nothing can read, and the first person to notice is the one
who needed it.

Two things are deliberate. `exec` records only the environment a command
*added*, never the ambient one: this file gets attached to bug reports and a
release machine's environment holds tokens. And rotation prefers to keep the
runs that crashed -- those are the ones somebody still wants.
"""

from __future__ import annotations

import json
import os
import platform
import threading
import time
from contextlib import contextmanager
from datetime import UTC, datetime
from pathlib import Path
from typing import TYPE_CHECKING, Any

from .config import GateConfig
from .harnessschema import RunLogConfig
from .runhistory import free_gb, head_revision, rotate, tree_size
from .runlogschema import (
    ActionRun,
    Artifact,
    Exec,
    Note,
    Payload,
    PlanShape,
    RunEnd,
    RunStart,
    StepEnd,
    StepStart,
)

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .actions import Action
    from .execution import Step

OK, FAILED, SKIPPED = "ok", "failed", "skipped"
_GB = 1024**3


class RunLog:
    """The record of one gate run."""

    def __init__(self, root: Path, settings: RunLogConfig, *, command: str) -> None:
        self.settings = settings
        self.command = command
        self.run_id = f"{datetime.now(UTC):%Y%m%d-%H%M%S}-{command}"
        self.directory = root / self.run_id
        self._events = self.directory / settings.events
        self._steps = self.directory / settings.step_log_dir
        # Belt and braces. Each emit opens, writes one line, and closes, and
        # an `O_APPEND` write to a regular file is already atomic -- no test
        # here can be made to fail without this lock. It stays because the
        # atomicity is a property of the current write shape rather than of
        # the interface, and the day someone holds the handle open across
        # several writes this is what stops the log shredding itself.
        #
        # A mutex, not a scheduler: it creates no concurrency of its own.
        self._writing = threading.Lock()
        self._started = time.monotonic()
        self._current = ""

    # -- opening and closing -----------------------------------------------

    @classmethod
    @contextmanager
    def open(cls, config: GateConfig, command: str, *, argv: tuple[str, ...] = ()):
        """A run's directory, for the length of that run."""
        settings = config.runlog
        root = config.path(settings.root)
        log = cls(root, settings, command=command)
        log._begin(config, argv)
        try:
            yield log
        except BaseException as error:
            log.close(FAILED, failures={command: str(error)})
            raise
        else:
            log.close(OK)

    def _begin(self, config: GateConfig, argv: tuple[str, ...]) -> None:
        self._steps.mkdir(parents=True, exist_ok=True)
        rotate(config, keep=self.directory)
        self._point_latest_here()
        self.emit(
            RunStart(
                command=self.command,
                argv=argv,
                head=head_revision(config.root),
                platform=platform.system(),
                machine=platform.machine(),
                cores=os.cpu_count() or 0,
                free_gb=free_gb(config.root),
            )
        )

    def close(self, status: str, **summary: Any) -> None:
        self.emit(
            RunEnd(
                status=status,
                duration_ms=(time.monotonic() - self._started) * 1000,
                footprint_gb=tree_size(self.directory) / _GB,
                **summary,
            )
        )

    def _point_latest_here(self) -> None:
        """So `runs last` and a bug report have one path to name."""
        latest = self.directory.parent / self.settings.latest_link
        if latest.is_symlink() or latest.exists():
            latest.unlink()
        latest.symlink_to(self.directory.name)

    # -- writing -----------------------------------------------------------

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

    def step_log(self, label: str) -> Path:
        """Where a step's own output goes, so concurrent lanes stay readable."""
        return self._steps / f"{label}.log"

    # -- the Journal an action writes to -----------------------------------

    def note(self, message: str) -> None:
        self.emit(Note(step=self._current, message=message))

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        self.emit(
            Artifact(step=self._current, path=str(path), size=size, digest=digest)
        )

    def exec(
        self, argv: tuple[str, ...], *, cwd: str, env: dict[str, str], exit: int, duration_ms: float
    ) -> None:
        self.emit(
            Exec(
                step=self._current,
                argv=argv,
                cwd=cwd,
                env=env,
                exit=exit,
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
        previous, self._current = self._current, step.label
        try:
            yield
        except BaseException as error:
            self._end_step(step.label, FAILED, started, error)
            raise
        else:
            self._end_step(step.label, OK, started, None)
        finally:
            self._current = previous

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
        self.emit(
            ActionRun(
                step=self._current,
                action=action.name,
                render=action.render(),
                duration_ms=(time.monotonic() - started) * 1000,
                status=status,
            )
        )

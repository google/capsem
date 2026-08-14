"""One directory per run, so a failed gate is something you can attach.

Before this, diagnosing a gate failure meant having been present when it
happened. Which command ran with which arguments, what it exited with, how long
each phase took, which bytes came out -- all of it existed only as terminal
scrollback, and only for whoever was watching.

This is what a *run* is: where its directory comes from, what protects it while
it is live, and what closes it. How an individual event is written down, and
the `step` and `action` brackets that produce them, are `journal`.

Two things are deliberate. `exec` records only the environment a command
*added*, never the ambient one: this file gets attached to bug reports and a
release machine's environment holds tokens. And rotation prefers to keep the
runs that crashed -- those are the ones somebody still wants.
"""

from __future__ import annotations

import os
import platform
import secrets
import time
from contextlib import contextmanager
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from capsem.gatelaunch import PYCACHE

from . import digestreport, runledger
from .config import GateConfig
from .harnessschema import RunLogConfig
from .journal import _CURRENT, FAILED, OK, EventJournal
from .runhistory import (
    free_gb,
    head_revision,
    history_locked,
    hold_active,
    point_latest,
    release_active,
    rotate,
    tree_size,
)
from .runlogschema import RunEnd, RunStart
from .sourcecommit import SourceCommit
from .sourcestate import gate_source
from .summary import write_summary

_GB = 1024**3


class RunLog(EventJournal):
    """The record of one gate run."""

    def __init__(self, root: Path, settings: RunLogConfig, *, command: str) -> None:
        # A short random suffix, because the id had one-second resolution and
        # the machine lock is taken *after* the log is opened -- so two
        # contenders arriving together collided on the way in, and each
        # rotation then protected only its own path.
        run_id = f"{datetime.now(UTC):%Y%m%d-%H%M%S}-{secrets.token_hex(3)}-{command}"
        self.directory = root / run_id
        super().__init__(self.directory / settings.events, settings, run_id=run_id)
        self.command = command
        self._steps = self.directory / settings.step_log_dir
        self._started = time.monotonic()
        self._qualification_commit: SourceCommit | None = None

    # -- opening and closing -----------------------------------------------

    @classmethod
    @contextmanager
    def open(
        cls,
        config: GateConfig,
        command: str,
        *,
        argv: tuple[str, ...] = (),
        source_commit: str | None = None,
    ):
        """A run's directory, for the length of that run."""
        settings = config.runlog
        root = config.path(settings.root)
        log = cls(root, settings, command=command)
        log._begin(config, argv, source_commit)
        try:
            yield log
        except BaseException as error:
            log.close(FAILED, failures={command: str(error)})
            raise
        else:
            log.close(OK)

    def _begin(self, config: GateConfig, argv: tuple[str, ...], source_commit: str | None) -> None:
        # Kept for `close`, which has to record this run in the ledger and
        # regenerate the digest, and both are questions about the whole
        # history rather than about this directory.
        self._config = config
        # Everything that makes this directory visible, and everything that
        # could act on another one, inside a single hold of the history lock.
        # The comment here used to claim the marker was taken before anything
        # could see the directory while the code created the directory first,
        # leaving exactly the window it described: another allocator arriving
        # in it sees an unmarked, unfinished run and rotates it away.
        with history_locked(config):
            self._steps.mkdir(parents=True, exist_ok=True)
            self._active = hold_active(self.directory, self.settings)
            rotate(config, keep=self.directory)
            point_latest(self.directory, self.settings)
        self.emit(
            RunStart(
                command=self.command,
                argv=argv,
                head=head_revision(config.root),
                source_commit=source_commit,
                platform=platform.system(),
                machine=platform.machine(),
                cores=os.cpu_count() or 0,
                free_gb=free_gb(config.root),
                # What ran, beside what it says it tested.
                gate_source=str(gate_source()),
                pycache=os.environ.get(PYCACHE, ""),
            )
        )

    def close(self, status: str, **summary: Any) -> None:
        """Held until the last byte is written, released on every path.

        The marker used to be dropped first, so a command waiting to allocate
        could rotate this directory away while its owner was still writing the
        terminal event and the summary into it. Under a tight byte cap that
        turns a release which had already published into a logging failure.
        """
        try:
            if self._qualification_commit is not None:
                from .qualificationevidence import archive_attempt

                archive_attempt(self._config, self._qualification_commit, self.directory)
            self.emit(
                RunEnd(
                    status=status,
                    duration_ms=(time.monotonic() - self._started) * 1000,
                    footprint_gb=tree_size(self.directory) / _GB,
                    **summary,
                )
            )
            write_summary(self.directory, self.settings, command=self.command, run_id=self.run_id)
            self._record_history()
        finally:
            self._active = release_active(self._active)

    def qualification_attempt(self, commit: SourceCommit) -> None:
        """Retain this exact-source journal independently of run rotation."""
        self._qualification_commit = commit

    def _record_history(self) -> None:
        """Add this run to the ledger and rewrite the digest.

        Best-effort *here* and enforced elsewhere, deliberately. `close` runs
        on the failure path too, and a bookkeeping error raised from it would
        replace the failure somebody actually needs to read -- the lifecycle
        rule that a primary error survives cleanup. So a problem is written
        into this run's own log and the run keeps its real outcome.

        That would be an invisible degradation on its own, which is why the
        fast phase regenerates the digest as an ordinary step that is allowed
        to fail. Between the two, a broken ledger is caught in seconds without
        ever being able to eat a failure.
        """
        config = getattr(self, "_config", None)
        if config is None:
            return
        try:
            runledger.append(config, self.directory, self.settings)
            digestreport.write(config)
        except Exception as error:
            self.note(f"run history not updated: {error}")

    def step_log(self, label: str) -> Path:
        """Where a step's own output goes, so concurrent lanes stay readable."""
        return self._steps / f"{label}.log"

    def step_output(self) -> Path | None:
        """The running step's log, or nothing outside a step.

        Resources acquire and release outside the step graph, so there are
        real commands with no step to file under. Refusing to run those would
        be a worse answer than not filing their output.
        """
        label = _CURRENT.get()
        return self.step_log(label) if label else None

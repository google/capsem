"""Whether one invocation writes a run of its own, and what it reports.

Split from `command`, which is about what a command *is* -- what it holds, what
it contains, and the order `execute` puts those in. This is the other question:
does asking count as doing.

`runs last --failed` once opened a run and repointed `latest` at itself before
answering, so the honest answer to "which run failed" could be the question.
Inspection commands therefore opt out explicitly while every command that can
change the machine records by default.

A mixin rather than free functions: every one of these reads the command's own
configuration, arguments and invocation, and threading four of those through a
module boundary would be ceremony around a question that belongs to the object.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from typing import ClassVar

from .config import GateConfig
from .context import NullJournal
from .qualificationevidence import QualificationEvidence
from .runhistory import read
from .runlog import RunLog
from .runlogschema import QualificationReuse
from .sourcecommit import SourceCommit
from .timing import measure, report


@contextmanager
def _no_record():
    """A journal for a command that must not leave a run behind."""
    yield NullJournal()


class Recorded:
    """The recording half of a command's lifecycle."""

    name: ClassVar[str]
    """Supplied by `GateCommand`; declared here so the mixin type-checks."""

    _config: GateConfig
    _args: argparse.Namespace
    _invocation: tuple[str, ...]

    records: ClassVar[bool] = True
    """Whether this command writes a run of its own.

    False for the ones that only *read* runs. Asking must not become part of
    what is being asked about.
    """

    def should_record(self) -> bool:
        """Whether this invocation writes a run of its own."""
        return self.records

    @contextmanager
    def _recording(self, *, source_commit: str | None = None):
        """The run log, or a journal that keeps nothing.

        A command that only reads runs must not create one; everything else
        below is identical either way, which is the point.
        """
        if not self.should_record():
            with _no_record() as log:
                yield log
            if self._args.timing:
                self._summarize(log)
            return

        recorded = None
        try:
            with RunLog.open(
                self._config,
                self.name,
                argv=self._argv(),
                source_commit=source_commit,
            ) as log:
                recorded = log
                yield log
        finally:
            if isinstance(recorded, RunLog):
                self._summarize(recorded)

    def _summarize(self, log: RunLog) -> None:
        """Say where the time went, on the way out.

        A command that recorded no run has no time to report. This assumed the
        journal always had a run directory, so `--timing` on any of the
        readers ended in `AttributeError: 'NullJournal' object has no
        attribute 'directory'` after printing the answer.
        """
        if not self.should_record():
            print(f"{self.name} records no run, so there is no timing to report")
            return
        timing = measure(read(log.directory, self._config.runlog))
        print(
            report(
                timing,
                command=self.name,
                settings=self._config.runlog,
                run_id=log.run_id,
            )
        )

    def _record_qualification_reuse(
        self, commit: SourceCommit, evidence: QualificationEvidence
    ) -> None:
        """Return ordinary success while retaining exactly why no work ran."""
        from .execution import Kind, Speed, step

        with self._recording(source_commit=str(commit)) as log:
            if not isinstance(log, RunLog):
                raise TypeError("qualification reuse requires a recorded command")
            label = "qualification.reuse"
            log.shape((label,), ())
            log.emit(
                QualificationReuse(
                    source_commit=str(commit),
                    qualification=evidence.reference,
                )
            )
            with log.step(step(label, kind=Kind.STATIC_TEST, speed=Speed.FAST)):
                log.note(
                    f"source {commit} already qualified by {evidence.reference.run_id}; "
                    f"{evidence.reference.run_log} ({evidence.reference.digest})"
                )
        print(
            f"{commit} is already qualified by {evidence.reference.run_id}: "
            f"{evidence.reference.run_log} ({evidence.reference.digest})"
        )

    def _argv(self) -> tuple[str, ...]:
        """The logical invocation, or the command name when built directly."""
        return self._invocation or (self.name,)

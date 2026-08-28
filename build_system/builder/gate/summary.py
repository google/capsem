"""The human-readable half of a run, written once its events are on disk.

Separate from `runlog`, which writes events. This *reads* a finished run and
renders it, which is the same thing `--timing` and `runs show` do -- and
keeping it beside the writer pushed that module past the size the boundary
guard holds.

Written for every run rather than only when asked, so a run nobody enquired
about still leaves something a bug report can attach. That is exactly the run
that most needs one.
"""

from __future__ import annotations

from pathlib import Path

from .harnessschema import RunLogConfig


def write_summary(directory: Path, settings: RunLogConfig, *, command: str, run_id: str) -> None:
    from .runhistory import read
    from .timing import measure, report

    rendered = report(
        measure(read(directory, settings)),
        command=command,
        settings=settings,
        run_id=run_id,
    )
    (directory / settings.summary).write_text(rendered, encoding="utf-8")

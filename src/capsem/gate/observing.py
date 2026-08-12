"""Wiring the run's filesystem observer into the funnel.

Its own module because `command.py` is the enforcing funnel and every line
added there is a line every command pays for. Three destinations, because they
answer different questions and the first two were the ones missing: `stderr`
so a developer sees the fault at the minute it occurs rather than at the end
of an hour, a size-capped log beside the run so a killed gate still leaves the
report, and the journal so `runs` can answer it later.
"""

from __future__ import annotations

import sys
import threading
from collections.abc import Iterator
from contextlib import contextmanager

from .config import GateConfig
from .faultlog import FaultLog
from .faults import Fault
from .observation import SOURCE_TREE, Watch
from .plan import Plan


@contextmanager
def observing(
    config: GateConfig, log: object, plan: Plan, *, publishes: bool = False
) -> Iterator[Watch | None]:
    """Watch the disk for the length of a run, reporting faults as they land.

    `publishes` is what separates a report from a refusal. A source-tree fault
    means the tree changed under a run that is measuring it -- tolerable for a
    developer, who can read the log and decide, and not tolerable for a command
    that is about to publish an artifact whose provenance says it was built from
    a tree that held still."""
    from .interception import Instrument

    # Only a real run has somewhere to put the report. A test driving the
    # funnel with a recording journal is not being audited, and inventing a
    # directory for it would drop fault files into the checkout.
    directory = getattr(log, "directory", None)
    if directory is None:
        yield None
        return

    settings = config.runlog
    errors = FaultLog(
        directory / settings.error_log,
        max_bytes=settings.error_log_max_bytes,
        keep=settings.error_log_keep,
    )
    seen: list[Fault] = []
    reporting = threading.Lock()

    def report(fault: Fault) -> None:
        fatal = publishes and fault.reason == SOURCE_TREE

        def record() -> None:
            seen.append(fault)
            level = "FATAL" if fatal else "OBSERVATION"
            print(f"HERMETICITY {level} {fault.render()}", file=sys.stderr, flush=True)
            errors(fault)
            note = getattr(log, "note", None)
            if note is not None:
                note(f"fault {fault.reason}: {fault.path}")

        with reporting:
            if not fatal:
                record()
                return
            watch.refuse_after(
                f"the source tree changed during a release: {fault.render()}. "
                "This run would publish an artifact whose recorded provenance "
                "claims a tree that did not hold still, and no later check can "
                "tell the difference. Re-run with the checkout left alone.",
                record,
            )

    declared = {
        step.label: frozenset(resource.name for resource in step.contends) for step in plan.steps
    }
    roots = [config.path(name) for name in settings.observed_roots]
    watch = Watch(roots, source_root=config.root, declared=declared, on_fault=report)
    try:
        with watch, Instrument(watch, fd_path_template=settings.fd_path_template):
            yield watch
        # After the observer is stopped, so no callback can race the final
        # sweep/checkpoint and land after a publishing plan was accepted.
        watch.sweep()
        watch.checkpoint()
    finally:
        errors.close()
        if seen:
            print(
                f"{len(seen)} filesystem fault(s) -- see {errors.path}",
                file=sys.stderr,
                flush=True,
            )

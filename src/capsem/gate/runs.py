"""Reading a finished run back: what it did, how long, and what broke.

The run log is only worth writing if something reads it, and the reading is
where the value lands. A red gate on someone else's machine used to arrive as
a screenshot of a terminal; it now arrives as a directory, and this is what
turns that directory back into an answer.

None of these take the machine lock. Asking what a run did is a question you
should be able to ask *while* the next one is going.
"""

from __future__ import annotations

from pathlib import Path

from . import digestreport
from .actions import Call
from .command import GateCommand
from .context import Context
from .errors import GateError
from .execution import Kind, Needs, Speed, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .runhistory import read, runs
from .runledger import comparable_to, containing, rows
from .timing import clock, measure, report


class RunsCommand(GateCommand, name="runs", help="list recorded gate runs, or explain one"):
    records = False
    """Only reads runs; creating one would answer with the question."""

    @classmethod
    def add_arguments(cls, parser) -> None:
        actions = parser.add_subparsers(dest="action", required=False)

        show = actions.add_parser("show", help="explain one run")
        show.add_argument("run", help="a run id, or `latest`")

        last = actions.add_parser("last", help="explain the most recent run")
        last.add_argument(
            "--failed",
            action="store_true",
            help="the most recent run that failed, rather than the most recent",
        )

        actions.add_parser("digest", help="the cross-run overview, regenerated")

        trend = actions.add_parser("trend", help="one step's history across comparable runs")
        trend.add_argument(
            "--step",
            default="",
            help="a step label; omitted, the critical path of the latest run",
        )

    def plan(self) -> Plan:
        action = getattr(self._args, "action", None) or "list"
        plan = Plan(f"{self.name} {action}")
        plan.add(
            step(
                action,
                Call(
                    f"runs {action}",
                    self._operation(action),
                    justification=CallJustification(
                        kind=OpaqueKind.RUNTIME_DERIVED,
                        reason="which run to read and what it contains is a question about the history on disk",
                        effects=machine_effects(Effect.FILESYSTEM),
                    ),
                ),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            )
        )
        return plan

    def _operation(self, action: str):
        if action == "show":
            return lambda ctx: _explain(ctx, self._resolve(ctx, self._args.run))
        if action == "last":
            return lambda ctx: _explain(ctx, self._latest(ctx, failed=self._args.failed))
        if action == "digest":
            return _digest
        if action == "trend":
            return lambda ctx: _trend(ctx, self._args.step)
        return _list

    def _resolve(self, context: Context, wanted: str) -> Path:
        root = context.path(context.config.runlog.root)
        candidate = root / wanted
        if candidate.is_dir():
            return candidate.resolve()
        raise GateError(f"no run called {wanted!r} under {root}")

    def _latest(self, context: Context, *, failed: bool) -> Path:
        recorded = runs(context.config)
        if not recorded:
            raise GateError("no runs have been recorded yet")
        if not failed:
            return recorded[0]

        for directory in recorded:
            # `outcome`, so a run that failed while taking the lock or
            # releasing a resource is reachable. Selecting on failed *steps*
            # skipped exactly the runs whose failure was hardest to diagnose.
            if measure(read(directory, context.config.runlog)).outcome == "failed":
                return directory
        raise GateError("no recorded run failed; the most recent is " + recorded[0].name)


def _list(context: Context) -> None:
    recorded = runs(context.config)
    if not recorded:
        context.runner.note("no runs recorded yet")
        return

    for directory in recorded:
        timing = measure(read(directory, context.config.runlog))
        state = "FAILED" if timing.outcome == "failed" else "ok"
        context.runner.note(f"{directory.name:<34}  {timing.total_ms / 1000:>8.0f}s  {state}")


def _digest(context: Context) -> None:
    """Rewrite the overview and print it.

    Regenerated rather than read back, so asking cannot return a document that
    disagrees with the ledger. This is also the command the fast phase runs and
    the one an agent session is handed, and all three must produce the same
    text or the digest becomes another thing to reconcile.
    """
    context.runner.note(digestreport.write(context.config).read_text(encoding="utf-8"))


def _trend(context: Context, label: str) -> None:
    """One step, run by run, across everything comparable in the ledger.

    The digest names hotspots; this is for reading the shape of one. A step
    that alternates between eight seconds and four minutes is a different
    problem from one that has drifted upward, and a median hides both.
    """
    history = rows(context.config)
    if not history:
        raise GateError("the ledger is empty; run the gate once")

    latest = history[0]
    limit = context.config.runlog.digest.compare_runs
    if label:
        window = containing(history, label, limit)
        if not window:
            raise GateError(f"no recorded run has a step called {label!r}")
        wanted = [label]
    else:
        window = [latest, *comparable_to(latest, history, limit)]
        wanted = list(latest.critical_path)
        if not wanted:
            raise GateError(f"{latest.run_id} recorded no critical path to follow")

    for name in wanted:
        context.runner.note(f"\n{name}")
        for row in reversed(window):
            # A run where the step did not do the work is shown with its
            # status rather than dropped: a step skipped half the time is the
            # finding, and hiding those rows is what makes it invisible.
            step = row.steps.get(name)
            measured = row.measured(name)
            rendered = (
                clock(measured) if measured is not None else (step.status if step else "absent")
            )
            context.runner.note(f"  {row.run_id:<34}  {rendered:>9}  {row.command}")


def _explain(context: Context, directory: Path) -> None:
    events = read(directory, context.config.runlog)
    if not events:
        raise GateError(f"{directory} has no recorded events")

    context.runner.note(
        report(
            measure(events),
            command=directory.name,
            settings=context.config.runlog,
            run_id=directory.name,
        )
    )

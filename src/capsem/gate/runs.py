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

from .actions import Call, Why
from .command import GateCommand
from .context import Context
from .errors import GateError
from .execution import step
from .plan import Plan
from .runhistory import read, runs
from .timing import measure, report


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

    def plan(self) -> Plan:
        action = getattr(self._args, "action", None) or "list"
        plan = Plan(f"{self.name} {action}")
        plan.add(
            step(
                action,
                Call(f"runs {action}", self._operation(action), why=Why.DYNAMIC),
            )
        )
        return plan

    def _operation(self, action: str):
        if action == "show":
            return lambda ctx: _explain(ctx, self._resolve(ctx, self._args.run))
        if action == "last":
            return lambda ctx: _explain(ctx, self._latest(ctx, failed=self._args.failed))
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

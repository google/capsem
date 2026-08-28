"""Where a run's time went, computed from its recorded events.

"The gate is slow" has never been actionable, and the obvious answer -- rank
the steps and look at the top -- is usually wrong. Shortening the slowest step
changes nothing if it runs beside something longer. What a run's duration is
actually made of is its **critical path**: the longest chain of steps that had
to happen in order. That is the only thing shortening reliably helps.

Computed from the events rather than from a live `Plan`, because the question
is usually asked about a run that is already over -- often on another machine,
from a directory attached to a bug report. The plan's shape is in the log for
exactly this reason.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .harnessschema import RunLogConfig


@dataclass
class Timing:
    """What a run spent, and on what."""

    total_ms: float = 0.0
    steps: dict[str, float] = field(default_factory=dict)
    status: dict[str, str] = field(default_factory=dict)
    actions: list[tuple[str, str, float]] = field(default_factory=list)
    """`(step, render, duration_ms)`, so a slow line names itself."""

    critical_path: list[str] = field(default_factory=list)
    failures: dict[str, str] = field(default_factory=dict)
    skipped: list[str] = field(default_factory=list)

    run_failures: dict[str, str] = field(default_factory=dict)
    """What failed *outside* the steps, as `run.end` recorded it.

    Kept so the summary can name a lock, a resource or a teardown rather than
    only colouring the line red. A run whose every step passed and whose
    workspace then refused to release has nothing in `failures`, and without
    this there is nothing to print either.
    """

    dependency_waits: dict[str, float] = field(default_factory=dict)
    """How long each step sat after becoming runnable, measured from run start.

    The scheduling signal, and it was being thrown away. `step.waits` carries
    it on every run; only `resource_ms` was kept, so the one number that says
    "this step could have started much earlier" never reached the ledger and
    no analysis could ask.
    """

    resource_waits: dict[str, float] = field(default_factory=dict)
    """How long each step sat with its dependencies met and its claims held
    by somebody else.

    The number to look at before shortening anything: a plan whose critical
    path is mostly waiting is a contention problem, and making the steps
    themselves faster does not touch it.
    """

    recorded_status: str = "ok"
    """What `run.end` said, as distinct from what the steps said.

    A run can fail outside every step: the machine lock, a resource that would
    not acquire, a teardown that raised. `status` is the per-step map; this is
    the run's own outcome, and reading only the former reported a run whose
    every step passed and whose workspace then refused to release as a success.
    """

    @property
    def outcome(self) -> str:
        """`failed` if the run failed, by any route."""
        if self.recorded_status != "ok" or self.failures:
            return "failed"
        return "ok"

    @property
    def critical_ms(self) -> float:
        return sum(self.steps.get(label, 0.0) for label in self.critical_path)

    def slowest_actions(self, limit: int) -> list[tuple[str, str, float]]:
        return sorted(self.actions, key=lambda entry: -entry[2])[:limit]


def measure(events: list[dict]) -> Timing:
    """Read a recorded run and work out where its time went."""
    timing = Timing()
    edges: dict[str, set[str]] = {}
    order: list[str] = []

    for event in events:
        kind = event["event"]
        if kind == "plan":
            order = list(event["steps"])
            edges = {label: set() for label in order}
            for before, after in event["edges"]:
                edges.setdefault(after, set()).add(before)
        elif kind == "step.end":
            timing.steps[event["step"]] = event["duration_ms"]
            timing.status[event["step"]] = event["status"]
            if event["status"] == "failed":
                timing.failures[event["step"]] = event.get("error") or ""
            elif event["status"] == "skipped":
                timing.skipped.append(event["step"])
        elif kind == "step.waits":
            timing.resource_waits[event["step"]] = event["resource_ms"]
            timing.dependency_waits[event["step"]] = event.get("dependency_ms", 0.0)
        elif kind == "action":
            timing.actions.append((event["step"], event["render"], event["duration_ms"]))
        elif kind == "run.end":
            timing.total_ms = event["duration_ms"]
            # Defaulted, because a truncated log is a real case: a killed
            # gate leaves the events it managed to write, and reading one
            # should degrade rather than raise.
            timing.recorded_status = event.get("status", "ok")
            timing.run_failures = dict(event.get("failures") or {})

    timing.critical_path = longest_chain(order, edges, timing.steps)
    return timing


def longest_chain(
    order: list[str], edges: dict[str, set[str]], spent: dict[str, float]
) -> list[str]:
    """The slowest path through the graph, by measured duration.

    A single pass in topological order is enough: every step's dependencies
    are settled before it is reached, so the best chain ending at it is the
    best chain ending at one of them, plus itself.
    """
    if not order:
        return []

    best: dict[str, tuple[float, list[str]]] = {}
    for label in order:
        prior = max(
            (best[earlier] for earlier in edges.get(label, ()) if earlier in best),
            default=(0.0, []),
        )
        best[label] = (prior[0] + spent.get(label, 0.0), [*prior[1], label])

    return max(best.values(), key=lambda entry: entry[0])[1] if best else []


def report(timing: Timing, *, command: str, settings: RunLogConfig, run_id: str) -> str:
    """The summary printed at the end of a run, and by `runs show`."""
    # `outcome`, not `failures`: the steps are only half of what a run can
    # fail at. Classifying by them reported a run whose workspace refused to
    # release as a success.
    status = "FAILED" if timing.outcome == "failed" else "ok"
    lines = [f"{command} -- {clock(timing.total_ms)} -- {status}", ""]

    if timing.critical_path:
        lines.append(f"critical path ({clock(timing.critical_ms)} of {clock(timing.total_ms)})")
        widest = max(len(label) for label in timing.critical_path)
        longest = max(timing.steps.get(name, 0.0) for name in timing.critical_path) or 1.0
        for label in timing.critical_path:
            spent = timing.steps.get(label, 0.0)
            bar = "#" * max(1, round(24 * spent / longest))
            lines.append(f"  {label:<{widest}}  {clock(spent):>9}  {bar}")
        lines.append("")

    queued = sorted(
        (
            (spent, label)
            for label, spent in timing.resource_waits.items()
            if spent >= settings.slow_action_seconds * 1000
        ),
        reverse=True,
    )[:5]
    if queued:
        lines.append("longest resource waits")
        lines += [f"  {clock(spent):>9}  {label}" for spent, label in queued]
        lines.append("")

    slow = [
        entry
        for entry in timing.slowest_actions(5)
        if entry[2] >= settings.slow_action_seconds * 1000
    ]
    if slow:
        lines.append("slowest actions")
        lines += [f"  {clock(spent):>9}  {render[:64]}  ({step})" for step, render, spent in slow]
        lines.append("")

    if timing.failures or timing.run_failures:
        lines.append("failed")
        lines += [f"  {step}  {error}" for step, error in sorted(timing.failures.items())]
        # Named, not merely counted: a run that failed outside its steps has
        # nothing in `failures`, and "FAILED" with no cause is a line an
        # operator cannot act on.
        lines += [
            f"  {where}  {error}  (outside the plan)"
            for where, error in sorted(timing.run_failures.items())
        ]
        if timing.skipped:
            lines.append(f"  (never ran: {', '.join(sorted(timing.skipped))})")
        lines.append("")

    lines.append(f"run log  {settings.root}/{run_id}")
    return "\n".join(lines)


def clock(milliseconds: float) -> str:
    seconds = milliseconds / 1000
    if seconds < 60:
        return f"{seconds:.1f}s"
    return f"{int(seconds // 60)}m{int(seconds % 60):02d}s"

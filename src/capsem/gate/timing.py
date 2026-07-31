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
        elif kind == "action":
            timing.actions.append((event["step"], event["render"], event["duration_ms"]))
        elif kind == "run.end":
            timing.total_ms = event["duration_ms"]

    timing.critical_path = _longest_chain(order, edges, timing.steps)
    return timing


def _longest_chain(
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
    status = "FAILED" if timing.failures else "ok"
    lines = [f"{command} -- {_clock(timing.total_ms)} -- {status}", ""]

    if timing.critical_path:
        lines.append(
            f"critical path ({_clock(timing.critical_ms)} of "
            f"{_clock(timing.total_ms)})"
        )
        widest = max(len(label) for label in timing.critical_path)
        longest = max(timing.steps.get(name, 0.0) for name in timing.critical_path) or 1.0
        for label in timing.critical_path:
            spent = timing.steps.get(label, 0.0)
            bar = "#" * max(1, round(24 * spent / longest))
            lines.append(f"  {label:<{widest}}  {_clock(spent):>9}  {bar}")
        lines.append("")

    slow = [
        entry
        for entry in timing.slowest_actions(5)
        if entry[2] >= settings.slow_action_seconds * 1000
    ]
    if slow:
        lines.append("slowest actions")
        lines += [
            f"  {_clock(spent):>9}  {render[:64]}  ({step})" for step, render, spent in slow
        ]
        lines.append("")

    if timing.failures:
        lines.append("failed")
        lines += [f"  {step}  {error}" for step, error in sorted(timing.failures.items())]
        if timing.skipped:
            lines.append(f"  (never ran: {', '.join(sorted(timing.skipped))})")
        lines.append("")

    lines.append(f"run log  {settings.root}/{run_id}")
    return "\n".join(lines)


def _clock(milliseconds: float) -> str:
    seconds = milliseconds / 1000
    if seconds < 60:
        return f"{seconds:.1f}s"
    return f"{int(seconds // 60)}m{int(seconds % 60):02d}s"

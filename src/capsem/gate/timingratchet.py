"""Fail complete qualification when measured work becomes unwieldy.

No duration is authored here or in config. The baseline is the latest
successful run with the same invocation, host class and typed plan shape; the
policy says only how much relative growth is tolerable and how many of that
baseline's slowest steps are worth guarding.
"""

from __future__ import annotations

from enum import StrEnum
from pathlib import Path

from .actions import Action
from .config import GateConfig
from .context import Context
from .errors import GateError
from .harnessschema import TimingRegressionConfig
from .runhistory import read, runs
from .runledger import identity, one_event
from .runlog import RunLog
from .runlogschema import OK, PlanShape, RunEnd, RunStart
from .timing import Timing, longest_chain, measure


class TimingBoundary(StrEnum):
    """A typed graph frontier where elapsed work becomes release evidence."""

    QUALIFICATION = "source.verify"


class EnforceTimingRegression(Action, name="timing-ratchet"):
    """Compare completed qualification before release work can begin."""

    def __init__(self, boundary: TimingBoundary) -> None:
        self._boundary = boundary

    def render(self) -> str:
        return "ratchet qualification timing against comparable successful evidence"

    def perform(self, context: Context) -> None:
        # Unit-plan recording journals deliberately have no on-disk history.
        # Production commands always carry the concrete RunLog opened by the
        # execution funnel; only its validated evidence can seed a ratchet.
        if not isinstance(context.journal, RunLog):
            return
        baseline = enforce_current(context.config, context.journal.directory, self._boundary)
        message = (
            "timing ratchet seeded by this run"
            if baseline is None
            else f"timing ratchet passed against {baseline}"
        )
        context.journal.note(message)


def comparable(
    current: RunStart,
    current_shape: PlanShape,
    prior: RunStart,
    prior_shape: PlanShape,
) -> bool:
    """Whether elapsed-time evidence describes the same work and host class.

    Delegated, because the digest asks exactly this about ledger rows. Spelled
    out in two places they were free to disagree about a field, and the
    disagreement would surface as a release refused or allowed for a reason
    nobody could locate.
    """
    return identity(current, current_shape) == identity(prior, prior_shape)


def enforce_regression(
    current: Timing,
    baseline: Timing,
    shape: PlanShape,
    policy: TimingRegressionConfig,
    *,
    baseline_run: str,
) -> None:
    """Refuse relative growth in the critical path or ranked slow steps."""
    expected = set(shape.steps)
    if not expected <= set(current.steps) or not expected <= set(baseline.steps):
        raise GateError("timing ratchet evidence does not cover the typed plan shape")

    regressions: list[str] = []
    predecessors = {label: set() for label in shape.steps}
    for before, after in shape.edges:
        predecessors[after].add(before)
    current_path = longest_chain(list(shape.steps), predecessors, current.steps)
    baseline_path = longest_chain(list(shape.steps), predecessors, baseline.steps)
    _compare(
        regressions,
        "critical path",
        sum(current.steps[label] for label in current_path),
        sum(baseline.steps[label] for label in baseline_path),
        policy.maximum_factor,
    )
    ranked = sorted(shape.steps, key=lambda label: (-baseline.steps[label], label))
    for label in ranked[: policy.slowest_steps]:
        _compare(
            regressions,
            label,
            current.steps[label],
            baseline.steps[label],
            policy.maximum_factor,
        )

    if regressions:
        raise GateError(f"timing regression against {baseline_run}: " + "; ".join(regressions))


def _compare(
    regressions: list[str],
    label: str,
    current_ms: float,
    baseline_ms: float,
    maximum_factor: float,
) -> None:
    if baseline_ms <= 0:
        return
    factor = current_ms / baseline_ms
    if factor > maximum_factor:
        regressions.append(
            f"{label} {factor:.1f}x ({current_ms / 1000:.1f}s vs {baseline_ms / 1000:.1f}s)"
        )


def _before(shape: PlanShape, boundary: TimingBoundary) -> PlanShape:
    """The typed ancestor graph whose success the boundary certifies."""
    wanted = boundary.value
    if wanted not in shape.steps:
        raise GateError(f"timing boundary {wanted!r} is absent from the typed plan shape")
    predecessors = {label: set() for label in shape.steps}
    for before, after in shape.edges:
        predecessors[after].add(before)
    ancestors: set[str] = set()
    pending = list(predecessors[wanted])
    while pending:
        label = pending.pop()
        if label in ancestors:
            continue
        ancestors.add(label)
        pending.extend(predecessors[label])
    return PlanShape(
        steps=tuple(label for label in shape.steps if label in ancestors),
        edges=tuple(
            (before, after)
            for before, after in shape.edges
            if before in ancestors and after in ancestors
        ),
    )


def enforce_current(
    config: GateConfig,
    current_directory: Path,
    boundary: TimingBoundary,
) -> str | None:
    """Ratchet one finished plan against its latest comparable clean run."""
    current_events = read(current_directory, config.runlog)
    current_start = one_event(current_events, RunStart)
    current_shape = one_event(current_events, PlanShape)
    if current_start is None or current_shape is None:
        raise GateError("the current run lacks typed timing identity or plan shape")
    qualification_shape = _before(current_shape, boundary)

    for directory in runs(config):
        if directory == current_directory:
            continue
        events = read(directory, config.runlog)
        prior_start = one_event(events, RunStart)
        prior_shape = one_event(events, PlanShape)
        prior_end = one_event(events, RunEnd)
        if (
            prior_start is None
            or prior_shape is None
            or prior_end is None
            or prior_end.status != OK
            or not comparable(current_start, current_shape, prior_start, prior_shape)
        ):
            continue
        prior_timing = measure(events)
        if set(prior_timing.status.values()) != {OK}:
            continue
        enforce_regression(
            measure(current_events),
            prior_timing,
            qualification_shape,
            config.runlog.timing_regression,
            baseline_run=directory.name,
        )
        return directory.name
    return None

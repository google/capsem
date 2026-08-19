"""What the gate's recent history means, written for whoever reads it next.

`runs show` explains one run to someone who already knows what they are looking
for. This answers the question nobody was in a position to ask: what state is
this repository's build actually in, across the last several attempts, and what
should be done about it.

Written as much for a model as for a person. An agent picking up work here has
no memory of yesterday's run, and the failure modes that cost the most are
exactly the ones that need history to see -- a step that fails one run in four,
a phase that has doubled since a change three days ago, a critical path made of
queueing rather than work. Each of those reads as bad luck in isolation.

Every conclusion here is derived from measurements in the ledger, and every
threshold comes from `[runlog.digest]`. Nothing is asserted that a reader
cannot recompute from the same rows.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from statistics import median

from .digestschema import DigestConfig
from .runledger import LedgerRow, comparable_to
from .runlogschema import FAILED, OK
from .timing import clock


@dataclass(frozen=True)
class Trend:
    """One step measured against its own comparable history."""

    label: str
    current_ms: float
    median_ms: float
    samples: int

    @property
    def factor(self) -> float:
        return self.current_ms / self.median_ms if self.median_ms else 1.0


@dataclass(frozen=True)
class Hotspot:
    """A step that owns the critical path, and why it is long."""

    label: str
    residency: int
    window: int
    median_ms: float
    wait_share: float
    """How much of it was queueing rather than working."""


@dataclass(frozen=True)
class Thrash:
    """A step that fails without stopping the line."""

    label: str
    failures: int
    window: int


@dataclass
class Analysis:
    latest: LedgerRow | None = None
    recent: list[LedgerRow] = field(default_factory=list)
    baseline: list[LedgerRow] = field(default_factory=list)
    """Runs whose durations mean the same thing as the latest run's."""

    window: list[LedgerRow] = field(default_factory=list)
    """Recent runs of the same command, comparable or not.

    A wider net than `baseline`, and deliberately. Comparability is a
    requirement for *durations* -- a different graph measured different work.
    Whether a step keeps failing is not a timing question, and scoping it to
    identical plan shapes hid the most useful finding in the tree: a step that
    failed three times last week, across runs whose graphs had shifted, was
    reported as nothing at all.
    """

    regressions: list[Trend] = field(default_factory=list)
    improvements: list[Trend] = field(default_factory=list)
    hotspots: list[Hotspot] = field(default_factory=list)
    thrash: list[Thrash] = field(default_factory=list)

    @property
    def failing(self) -> bool:
        return self.latest is not None and self.latest.status != OK


def analyse(history: list[LedgerRow], settings: DigestConfig) -> Analysis:
    """Everything the digest reports, computed from rows alone."""
    if not history:
        return Analysis()

    latest = history[0]
    baseline = comparable_to(latest, history, settings.compare_runs)
    analysis = Analysis(
        latest=latest,
        recent=history[: settings.recent_runs],
        baseline=baseline,
        window=[row for row in history if row.command == latest.command][: settings.compare_runs],
    )
    if baseline:
        # Only durations need an identical graph. A first run of a new plan
        # shape has nothing to be measured against, and inventing a comparison
        # against a different graph is worse than declining to make one --
        # but it is no reason to stop counting failures.
        _trends(analysis, latest, baseline, settings)
    _hotspots(analysis, analysis.window, settings)
    # Every recent run, not just this command's. Three questions, three
    # scopes: a duration needs an identical graph, a critical path needs the
    # same command, and a step that keeps failing needs neither. Scoping this
    # like the other two buried the most useful thing in the tree -- a step
    # that had failed in three of the last candidate runs was invisible
    # because the newest run happened to be a fast test.
    _thrash(analysis, history[: settings.compare_runs], settings)
    return analysis


def _trends(
    analysis: Analysis, latest: LedgerRow, baseline: list[LedgerRow], settings: DigestConfig
) -> None:
    """Each step of the latest run against the median of its own history.

    Median rather than the single previous run, which is what the release
    ratchet uses. The ratchet is comparing one proof against one proof; a trend
    is trying to see through the variance of a shared machine, and one noisy
    neighbour makes a previous-run comparison fire constantly.

    Every measured step, and not only the critical path. This read
    `latest.critical_path or latest.steps` while claiming the sentence above,
    so on any run that had a path -- which is every real run -- a step that had
    tripled was invisible until it became the path itself. That is backwards:
    the whole value of a trend is seeing the growth *before* it is what the run
    waits for.

    Widening it costs nothing at the enforcement end, because nothing here
    enforces. It does cost a reader, so `trend_floor_seconds` drops the steps
    too short to be worth a sentence and `trends` bounds how many are named --
    this file is injected into every agent session, and an unbounded list of
    half-second wobbles is how a document stops being read.
    """
    floor_ms = settings.trend_floor_seconds * 1000
    for label in latest.steps:
        current = latest.measured(label)
        if current is None:
            continue
        # `measured` and not the raw duration: skipped and carried steps record
        # near-zero times, and a median over them says the work is free.
        samples = [spent for row in baseline if (spent := row.measured(label)) is not None]
        if not samples:
            continue
        trend = Trend(label, current, median(samples), len(samples))
        # Against the larger of the two, so a step that collapsed from a minute
        # to a moment is still reported. Testing `current` alone would drop
        # exactly the improvements worth confirming.
        if max(trend.current_ms, trend.median_ms) < floor_ms:
            continue
        if trend.factor >= settings.regression_factor:
            analysis.regressions.append(trend)
        elif trend.factor <= 1 / settings.improvement_factor:
            analysis.improvements.append(trend)
    analysis.regressions.sort(key=lambda t: -(t.current_ms - t.median_ms))
    analysis.improvements.sort(key=lambda t: t.current_ms - t.median_ms)
    del analysis.regressions[settings.trends :]
    del analysis.improvements[settings.trends :]


def _hotspots(analysis: Analysis, window: list[LedgerRow], settings: DigestConfig) -> None:
    """Steps that repeatedly own the critical path.

    Residency, not duration. The slowest step in a run is not worth shortening
    if it runs beside something longer, and the ranked-by-duration list that
    everyone reaches for first is exactly the list that keeps sending people to
    optimize work that was never on the path.
    """
    counted: dict[str, list[LedgerRow]] = {}
    for row in window:
        for label in row.critical_path:
            counted.setdefault(label, []).append(row)

    floor = max(1, round(settings.residency_fraction * len(window)))
    for label, owning in counted.items():
        if len(owning) < floor:
            continue
        spent = [value for row in owning if (value := row.measured(label)) is not None]
        if not spent:
            continue
        waits = [row.steps[label].resource_ms for row in owning if label in row.steps]
        typical = median(spent)
        analysis.hotspots.append(
            Hotspot(
                label=label,
                residency=len(owning),
                window=len(window),
                median_ms=typical,
                wait_share=(median(waits) / typical) if typical else 0.0,
            )
        )
    analysis.hotspots.sort(key=lambda h: -h.median_ms)
    del analysis.hotspots[settings.hotspots :]


def _thrash(analysis: Analysis, window: list[LedgerRow], settings: DigestConfig) -> None:
    """Steps that failed more than once across comparable runs.

    The expensive failure is not the one that stops the line; it is the one
    that fails often enough to be re-run and rarely enough to be excused. Those
    are invisible per-run by definition, which is why they are counted here.
    """
    counted: dict[str, int] = {}
    for row in window:
        for label, step in row.steps.items():
            if step.status == FAILED:
                counted[label] = counted.get(label, 0) + 1
    analysis.thrash = sorted(
        (
            Thrash(label, failures, len(window))
            for label, failures in counted.items()
            if failures >= settings.thrash_runs
        ),
        key=lambda t: -t.failures,
    )


def advice(analysis: Analysis, settings: DigestConfig) -> list[str]:
    """What the measurements imply, in the order worth acting on.

    Each line names the measurement it came from, so it can be disagreed with.
    Advice a reader cannot check is advice a reader has to either trust or
    ignore, and both are worse than an argument.
    """
    lines: list[str] = []
    if analysis.latest is None:
        return ["No runs recorded yet. The first `just test` seeds the ledger."]

    if analysis.failing:
        broken = sorted(
            label for label, step in analysis.latest.steps.items() if step.status == FAILED
        )
        lines.append(
            f"**The last run failed** at {', '.join(broken) or 'no step (it failed outside the plan)'}. "
            f"Read `capsem-gate runs show {analysis.latest.run_id}` before starting new work."
        )

    for thrash in analysis.thrash:
        lines.append(
            f"`{thrash.label}` failed in {thrash.failures} of the last {_runs(thrash.window)}. "
            "Intermittent, so it will be excused again unless it is fixed or quarantined deliberately."
        )

    for trend in analysis.regressions:
        lines.append(
            f"`{trend.label}` is {trend.factor:.1f}x its median "
            f"({clock(trend.current_ms)} against {clock(trend.median_ms)} over {_runs(trend.samples)}). "
            "If nothing about it changed, look at what it now waits on."
        )

    for hotspot in analysis.hotspots:
        if hotspot.wait_share >= settings.wait_fraction:
            lines.append(
                f"`{hotspot.label}` owns the critical path in {hotspot.residency} of {_runs(hotspot.window)} "
                f"and spends {hotspot.wait_share:.0%} of that queueing. Making it faster will not help; "
                "it is contending for a resource."
            )

    for trend in analysis.improvements:
        lines.append(
            f"`{trend.label}` is down to {trend.factor:.1f}x its median "
            f"({clock(trend.current_ms)} against {clock(trend.median_ms)}). "
            "If that was intentional, the median will follow it down."
        )

    if not analysis.baseline:
        # Said out loud, because the alternative is a digest that reports
        # "nothing anomalous" when what happened is that nothing was compared.
        # Absence of evidence printed as evidence of absence is the one failure
        # this document must not have.
        lines.append(
            "No prior run is comparable to this one -- the plan shape, arguments or host "
            f"class changed -- so **no durations were compared**. Counts below come from "
            f"{_runs(len(analysis.window))} of `{analysis.latest.command}`."
        )
    elif not lines:
        compared = len(analysis.baseline) + 1
        lines.append(
            f"Nothing anomalous across {compared} comparable runs. "
            "Hotspots below are where the time structurally goes, not a regression."
        )
    return lines


def _runs(count: int) -> str:
    return "1 run" if count == 1 else f"{count} runs"

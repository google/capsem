"""What the graph's shape costs, measured against what the runs recorded.

`rundigest` answers "is anything anomalous compared with last time". This
answers a different question: given this graph and what its steps actually
cost, is the schedule as short as the dependencies allow, and does each step
belong in the lane it is declared for.

Both need durations, so both must be honest when there are none. The rules are
the ones the digest already follows: per-step history rather than run-level
comparability, because a graph edit invalidates the latter exactly when the
analysis is most wanted; unmeasured steps named rather than assumed free; and
no claim at all rather than a claim nothing supports.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from statistics import median

from .execution import Speed
from .runledger import LedgerRow, containing
from .workgraph import WorkGraph


@dataclass(frozen=True)
class Slack:
    """How much later a step may start without lengthening the run."""

    node: str
    earliest_ms: float
    latest_ms: float

    @property
    def slack_ms(self) -> float:
        return self.latest_ms - self.earliest_ms

    @property
    def binding(self) -> bool:
        """Zero slack: this step is on a longest path and delaying it delays
        the run. The set of these is the real bottleneck, which is not the
        same as the list of slowest steps."""
        return self.slack_ms <= 0


@dataclass(frozen=True)
class LaneBreach:
    """A step whose measured cost contradicts the lane it declares."""

    node: str
    stage: str
    median_ms: float
    span_ms: float
    """The lane's critical path -- the shortest it could possibly run."""

    @property
    def share(self) -> float:
        return self.median_ms / self.span_ms if self.span_ms else 0.0


@dataclass
class Schedule:
    costs: dict[str, float] = field(default_factory=dict)
    unmeasured: list[str] = field(default_factory=list)
    slack: list[Slack] = field(default_factory=list)
    breaches: list[LaneBreach] = field(default_factory=list)

    @property
    def measurable(self) -> bool:
        """Whether anything here is worth reading.

        False on a first run, and said out loud rather than rendered as an
        empty findings list -- an empty list reads as "nothing wrong", which is
        the one thing this must not imply when the truth is "nothing known".
        """
        return bool(self.costs)


def costs_of(graph: WorkGraph, history: list[LedgerRow], window: int) -> Schedule:
    """Median measured duration per node, and which nodes have none.

    Per-step rather than per-run: `identity()` hashes the plan shape, so any
    edge change orphans every run-level baseline. `containing` finds runs that
    recorded a given step whatever else changed around it, which is the only
    reason this survives the graph being edited.
    """
    schedule = Schedule()
    for node in graph.nodes:
        samples = [
            spent
            for row in containing(history, node, window)
            if (spent := row.measured(node)) is not None
        ]
        if samples:
            schedule.costs[node] = median(samples)
        else:
            schedule.unmeasured.append(node)
    schedule.unmeasured.sort()
    return schedule


def analyse(
    graph: WorkGraph, history: list[LedgerRow], *, window: int, lane_share: float
) -> Schedule:
    """Slack over the graph, and lanes whose declaration does not hold."""
    schedule = costs_of(graph, history, window)
    if not schedule.measurable:
        return schedule
    _slack(graph, schedule)
    _lanes(graph, schedule, lane_share)
    return schedule


def _slack(graph: WorkGraph, schedule: Schedule) -> None:
    """Earliest and latest start for every node, by forward and backward pass.

    The standard two-pass computation over a DAG. Unmeasured nodes contribute
    zero, which is stated rather than hidden: they are listed in `unmeasured`,
    so a reader can see the slack numbers are a lower bound on the real ones.
    """
    predecessors = graph.predecessors()
    successors = graph.successors()
    order = _topological(graph)
    cost = schedule.costs

    earliest: dict[str, float] = {}
    for node in order:
        earliest[node] = max(
            (earliest[before] + cost.get(before, 0.0) for before in predecessors[node]),
            default=0.0,
        )
    span = max((earliest[node] + cost.get(node, 0.0) for node in order), default=0.0)

    latest: dict[str, float] = {}
    for node in reversed(order):
        latest[node] = min(
            (latest[after] - cost.get(node, 0.0) for after in successors[node]),
            default=span - cost.get(node, 0.0),
        )
    schedule.slack = sorted(
        (Slack(node, earliest[node], latest[node]) for node in order),
        key=lambda entry: entry.slack_ms,
    )


def _lanes(graph: WorkGraph, schedule: Schedule, lane_share: float) -> None:
    """`FAST` steps that own a large share of the run's critical path.

    Not an absolute second count. A two-minute step in a four-minute lane that
    guards a two-hour run is the trade that lane exists to make; the same two
    minutes elsewhere would be noise.

    Measured against the critical path rather than the sum of a stage's steps.
    The sum was tried first and produced nonsense: a `ty` pass at 2.6 seconds
    read as 63% of its stage, because that stage contains almost nothing else.
    The critical path is what the run actually costs, so a step's share of it
    is a step's share of the wait.
    """
    span = max(
        (entry.earliest_ms + schedule.costs.get(entry.node, 0.0) for entry in schedule.slack),
        default=0.0,
    )
    schedule.breaches = sorted(
        (
            LaneBreach(node.id, node.stage, schedule.costs[node.id], span)
            for node in graph.nodes.values()
            if node.speed is Speed.FAST
            and node.id in schedule.costs
            and span > 0
            and schedule.costs[node.id] / span >= lane_share
        ),
        key=lambda breach: -breach.share,
    )


def _topological(graph: WorkGraph) -> list[str]:
    """Node ids in an order the edges allow.

    Any linear extension will do -- both passes only need each node to come
    after its predecessors -- which is exactly why nothing here asserts on
    *which* order was chosen.
    """
    from graphlib import TopologicalSorter

    return list(TopologicalSorter(graph.predecessors()).static_order())

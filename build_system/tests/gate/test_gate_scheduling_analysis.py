"""Slack and lane share, and what they say when nothing has been measured.

The cold-start behaviour is the part worth testing. An analysis that returns an
empty finding list when it has no data reads as "nothing wrong", which is the
one thing it must never imply when the truth is "nothing known".
"""

from __future__ import annotations

from pathlib import Path

from capsem_builder.gate import scheduling
from capsem_builder.gate.execution import Arch, Kind, Speed
from capsem_builder.gate.runledger import LedgerRow, StepRow
from capsem_builder.gate.workgraph import Node, Origin, Requires, WorkGraph

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def _node(name: str, speed: Speed = Speed.FAST) -> Node:
    return Node(
        id=name,
        origin=Origin.GATE,
        stage="fast",
        kind=Kind.LINT,
        needs=frozenset(),
        arch=Arch.ANY,
        speed=speed,
        concurrency=1,
    )


def _chain() -> WorkGraph:
    """a -> b -> d, with c parallel to b. Only b is on the longest path."""
    return WorkGraph(
        nodes={name: _node(name) for name in ("a", "b", "c", "d")},
        edges={
            ("a", "b"): Requires.ORDER,
            ("a", "c"): Requires.ORDER,
            ("b", "d"): Requires.ORDER,
            ("c", "d"): Requires.ORDER,
        },
        conflicts=frozenset(),
    )


def _row(steps: dict[str, float]) -> LedgerRow:
    return LedgerRow(
        row_schema="test",
        run_id="20260101-000000-aaaaaa-candidate",
        command="candidate",
        head="0" * 40,
        status="ok",
        total_ms=sum(steps.values()),
        identity="same",
        critical_path=(),
        steps={name: StepRow(duration_ms=ms, status="ok") for name, ms in steps.items()},
    )


def test_nothing_measured_says_so_rather_than_finding_nothing() -> None:
    """The cold start. `measurable` is the flag a caller must check."""
    found = scheduling.analyse(_chain(), [], window=10, lane_share=0.25, queue_floor_ms=30_000)
    assert not found.measurable
    assert found.slack == [] and found.breaches == []
    assert found.unmeasured == ["a", "b", "c", "d"], (
        "every node must be named as unmeasured, not silently costed at zero"
    )


def test_an_unmeasured_step_is_named_rather_than_assumed_free() -> None:
    """Assuming zero makes a new expensive step invisible to the analysis
    written to find it -- the same bug as counting a skipped step as fast."""
    found = scheduling.analyse(
        _chain(),
        [_row({"a": 10.0, "b": 100.0, "d": 10.0})],
        window=10,
        lane_share=0.25,
        queue_floor_ms=30_000,
    )
    assert found.unmeasured == ["c"]
    assert "c" not in found.costs


def test_the_binding_set_is_the_longest_path_not_the_slowest_step() -> None:
    """`c` is slower than `b` but parallel to it, so it is not binding."""
    history = [_row({"a": 10.0, "b": 100.0, "c": 40.0, "d": 10.0})]
    found = scheduling.analyse(_chain(), history, window=10, lane_share=0.9, queue_floor_ms=30_000)
    binding = {entry.node for entry in found.slack if entry.binding}
    assert binding == {"a", "b", "d"}, f"expected the a->b->d chain, got {binding}"
    slack_of_c = next(entry for entry in found.slack if entry.node == "c")
    assert slack_of_c.slack_ms == 60.0, "c may start 60ms late without costing anything"


def test_lane_share_is_measured_against_the_critical_path() -> None:
    """Against the path, not the sum of a stage.

    The sum was tried and produced nonsense: a two-second step read as most of
    a stage that contained nothing else. The critical path is what the run
    actually waits for.
    """
    history = [_row({"a": 10.0, "b": 100.0, "c": 40.0, "d": 10.0})]
    found = scheduling.analyse(_chain(), history, window=10, lane_share=0.5, queue_floor_ms=30_000)
    named = {breach.node: breach for breach in found.breaches}
    assert set(named) == {"b"}, f"only b owns half the 120ms path, got {set(named)}"
    assert abs(named["b"].share - 100.0 / 120.0) < 1e-9

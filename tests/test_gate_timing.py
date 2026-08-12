"""Where a run's time went, and why the obvious answer is the wrong one.

"The gate is slow" was never actionable, and ranking the steps does not make
it so: shortening the slowest step changes nothing when it runs beside
something longer. A run's duration is made of its critical path -- the longest
chain that had to happen in order -- and that is the only thing shortening
reliably helps.

Computed from the recorded events rather than a live plan, because the
question is nearly always asked about a run that is already over, often from a
directory somebody attached to a bug report.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.errors import GateError
from capsem.gate.harnessschema import TimingRegressionConfig
from capsem.gate.runlog import RunLog
from capsem.gate.runlogschema import PlanShape, RunStart, StepEnd
from capsem.gate.timing import measure, report
from capsem.gate.timingratchet import (
    TimingBoundary,
    comparable,
    enforce_current,
    enforce_regression,
)

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SETTINGS = gate_config.load(PROJECT_ROOT).runlog


def _shape(steps, edges):
    return {"event": "plan", "steps": list(steps), "edges": [list(e) for e in edges]}


def _ended(step, ms, status="ok", error=None):
    return {
        "event": "step.end",
        "step": step,
        "duration_ms": ms,
        "status": status,
        "error": error,
    }


def _action(step, render, ms):
    return {"event": "action", "step": step, "render": render, "duration_ms": ms}


# ---------------------------------------------------------------------------
# The critical path
# ---------------------------------------------------------------------------


def test_the_critical_path_is_the_longest_chain_not_the_slowest_step() -> None:
    """The distinction that makes the number worth reporting.

    `lonely` is the slowest single step by a wide margin and is on nobody's
    path, so removing it entirely would not shorten the run by a second.
    """
    events = [
        _shape(
            ["prepare", "build", "verify", "lonely"],
            [("prepare", "build"), ("build", "verify")],
        ),
        _ended("prepare", 1_000),
        _ended("build", 2_000),
        _ended("verify", 1_500),
        _ended("lonely", 4_000),
    ]

    timing = measure(events)

    assert timing.critical_path == ["prepare", "build", "verify"]
    assert timing.critical_ms == 4_500


def test_the_longest_of_two_competing_chains_wins() -> None:
    events = [
        _shape(
            ["root", "slow", "quick", "join"],
            [("root", "slow"), ("root", "quick"), ("slow", "join"), ("quick", "join")],
        ),
        _ended("root", 100),
        _ended("slow", 900),
        _ended("quick", 50),
        _ended("join", 100),
    ]

    timing = measure(events)

    assert timing.critical_path == ["root", "slow", "join"]


def test_a_run_with_no_recorded_shape_still_reports_its_steps() -> None:
    """Runs written before the graph was recorded, and any run cut short
    before the plan was emitted, still have durations worth reading."""
    timing = measure([_ended("build", 1_000), _ended("verify", 500)])

    assert timing.critical_path == []
    assert timing.steps == {"build": 1_000, "verify": 500}


def test_a_step_that_never_ran_contributes_nothing_to_the_path() -> None:
    """A skipped step has no duration, and counting it as zero keeps it from
    distorting the chain it sits on."""
    events = [
        _shape(["stage", "install"], [("stage", "install")]),
        _ended("stage", 500, status="failed", error="boom"),
        _ended("install", 0, status="skipped"),
    ]

    timing = measure(events)

    assert timing.critical_ms == 500
    assert timing.skipped == ["install"]


# ---------------------------------------------------------------------------
# What the operator reads
# ---------------------------------------------------------------------------


def test_the_report_leads_with_the_outcome() -> None:
    events = [
        _shape(["build"], []),
        _ended("build", 500, status="failed", error="linker died"),
        {"event": "run.end", "duration_ms": 600},
    ]

    rendering = report(measure(events), command="test", settings=SETTINGS, run_id="r1")

    assert rendering.splitlines()[0].startswith("test")
    assert "FAILED" in rendering


def test_the_report_names_the_failure_and_what_it_took_down() -> None:
    """The blast radius is the second question anyone asks."""
    events = [
        _shape(["stage", "install"], [("stage", "install")]),
        _ended("stage", 500, status="failed", error="manifest missing"),
        _ended("install", 0, status="skipped"),
        {"event": "run.end", "duration_ms": 600},
    ]

    rendering = report(measure(events), command="test", settings=SETTINGS, run_id="r1")

    assert "manifest missing" in rendering
    assert "never ran" in rendering and "install" in rendering


def test_the_report_names_slow_actions_by_what_they_did() -> None:
    """ "run" would say nothing. The action renders itself for this."""
    events = [
        _shape(["assets"], []),
        _action("assets", "docker build -f docker/Dockerfile.rootfs .", 600_000),
        _ended("assets", 600_000),
        {"event": "run.end", "duration_ms": 600_000},
    ]

    rendering = report(measure(events), command="test", settings=SETTINGS, run_id="r1")

    assert "docker build" in rendering


def test_a_quick_action_is_not_worth_naming() -> None:
    """A list of every action is a list nobody reads."""
    events = [
        _shape(["lint"], []),
        _action("lint", "uv run ruff check .", 40),
        _ended("lint", 40),
        {"event": "run.end", "duration_ms": 50},
    ]

    rendering = report(measure(events), command="test", settings=SETTINGS, run_id="r1")

    assert "slowest actions" not in rendering


def test_the_report_points_at_the_run_it_describes() -> None:
    """So the next question -- "show me everything" -- has somewhere to go."""
    events = [_shape(["build"], []), _ended("build", 10), {"event": "run.end", "duration_ms": 20}]

    rendering = report(measure(events), command="test", settings=SETTINGS, run_id="r1")

    assert "r1" in rendering


def test_long_runs_are_reported_in_minutes() -> None:
    """`2312.4s` is a number people have to divide before they can react."""
    events = [
        _shape(["build"], []),
        _ended("build", 2_312_400),
        {"event": "run.end", "duration_ms": 2_312_400},
    ]

    rendering = report(measure(events), command="test", settings=SETTINGS, run_id="r1")

    assert "38m32s" in rendering


def test_a_passing_run_says_so_without_a_failure_section() -> None:
    events = [_shape(["build"], []), _ended("build", 100), {"event": "run.end", "duration_ms": 120}]

    rendering = report(measure(events), command="test", settings=SETTINGS, run_id="r1")

    assert "FAILED" not in rendering
    assert "failed" not in rendering.splitlines()


# ---------------------------------------------------------------------------
# Performance is a contract, not only a report
# ---------------------------------------------------------------------------


def _policy(*, factor: float = 1.5, slowest: int = 2) -> TimingRegressionConfig:
    return TimingRegressionConfig(maximum_factor=factor, slowest_steps=slowest)


def test_the_ratchet_policy_contains_no_authored_duration() -> None:
    assert set(TimingRegressionConfig.model_fields) == {"maximum_factor", "slowest_steps"}
    assert SETTINGS.timing_regression.maximum_factor == 1.5
    assert SETTINGS.timing_regression.slowest_steps == 10
    with pytest.raises(ValueError, match="greater than one"):
        _policy(factor=1.0)


def _start(*, head: str = "new", cores: int = 16) -> RunStart:
    return RunStart(
        command="candidate",
        argv=("capsem-gate", "candidate"),
        head=head,
        platform="Linux",
        machine="x86_64",
        cores=cores,
        free_gb=100.0,
        gate_source="/checkout/src/capsem/gate/__init__.py",
        pycache="/tmp/pycache",
    )


def test_comparison_uses_the_typed_plan_shape_and_host_class_not_the_head() -> None:
    shape = PlanShape(steps=("compile", "verify"), edges=(("compile", "verify"),))

    assert comparable(_start(), shape, _start(head="old"), shape)
    assert not comparable(_start(), shape, _start(cores=8), shape)
    assert not comparable(
        _start(),
        shape,
        _start(head="old"),
        PlanShape(steps=("compile",), edges=()),
    )


def test_an_unwieldy_step_regression_names_actual_baseline_and_factor() -> None:
    shape = PlanShape(steps=("prepare", "compile", "verify"), edges=())
    baseline = measure(
        [shape.model_dump(), _ended("prepare", 100), _ended("compile", 1_000), _ended("verify", 10)]
    )
    current = measure(
        [shape.model_dump(), _ended("prepare", 100), _ended("compile", 1_600), _ended("verify", 10)]
    )

    with pytest.raises(GateError, match=r"baseline-run.*compile.*1\.6x"):
        enforce_regression(
            current,
            baseline,
            shape,
            _policy(),
            baseline_run="baseline-run",
        )


def test_critical_path_regression_is_evidence_derived_not_authored_seconds() -> None:
    shape = PlanShape(steps=("prepare", "compile"), edges=(("prepare", "compile"),))
    baseline = measure([shape.model_dump(), _ended("prepare", 1_000), _ended("compile", 1_000)])
    current = measure([shape.model_dump(), _ended("prepare", 1_600), _ended("compile", 1_600)])

    with pytest.raises(GateError, match=r"critical path.*1\.6x"):
        enforce_regression(current, baseline, shape, _policy(), baseline_run="older")


def test_only_the_baselines_slowest_ranked_steps_are_ratcheted() -> None:
    shape = PlanShape(steps=("slow", "tiny"), edges=())
    baseline = measure([shape.model_dump(), _ended("slow", 1_000), _ended("tiny", 1)])
    current = measure([shape.model_dump(), _ended("slow", 1_499), _ended("tiny", 100)])

    enforce_regression(
        current,
        baseline,
        shape,
        _policy(slowest=1),
        baseline_run="older",
    )


def test_the_latest_comparable_successful_journal_is_the_baseline(tmp_path: Path) -> None:
    config_dir = tmp_path / "config"
    config_dir.mkdir()
    (config_dir / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    config = gate_config.load(tmp_path)
    invocation = ("capsem-gate", "candidate")
    boundary = TimingBoundary.QUALIFICATION
    shape = ("compile", boundary.value)
    edges = (("compile", boundary.value),)

    with RunLog.open(config, "candidate", argv=invocation) as baseline:
        baseline.shape(shape, edges)
        baseline.emit(StepEnd(step="compile", status="ok", duration_ms=1_000))
        baseline.emit(StepEnd(step=boundary.value, status="ok", duration_ms=1))

    with (
        pytest.raises(GateError, match=r"compile.*1\.6x"),
        RunLog.open(config, "candidate", argv=invocation) as current,
    ):
        current.shape(shape, edges)
        current.emit(StepEnd(step="compile", status="ok", duration_ms=1_600))
        enforce_current(config, current.directory, boundary)

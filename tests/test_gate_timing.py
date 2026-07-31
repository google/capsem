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

from capsem.gate import config as gate_config
from capsem.gate.timing import measure, report

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
    """"run" would say nothing. The action renders itself for this."""
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

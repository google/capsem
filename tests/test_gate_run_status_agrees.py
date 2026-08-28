"""Every consumer of a run agrees with what `run.end` recorded.

`Timing.outcome` has been right all along: a run can fail outside every step --
the machine lock, a resource that would not acquire, a teardown that raised --
and it treats a failed `run.end` as failed. Nothing read it.

    outcome: failed
    synthetic -- 0.1s -- ok

The summary, the run list and `runs last --failed` each classified a run by its
*steps*, so exactly the failures that happen outside a step were invisible in
the three places an operator looks for them. A resource failure would mark the
run failed and then be reported as a success.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.timing import Timing, measure, report

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def _events(*, run_status: str, step_status: str = "ok") -> list[dict]:
    """A run whose single step passed, ending however the caller says."""
    return [
        {"event": "plan", "steps": ["work"], "edges": []},
        {"event": "step.end", "step": "work", "status": step_status, "duration_ms": 100.0},
        {
            "event": "run.end",
            "status": run_status,
            "duration_ms": 100.0,
            "failures": {"workspace": "could not release"} if run_status != "ok" else {},
        },
    ]


def test_a_run_that_failed_outside_every_step_reports_as_failed() -> None:
    timing = measure(_events(run_status="failed"))

    assert timing.outcome == "failed"
    rendered = report(timing, command="synthetic", settings=CONFIG.runlog, run_id="r")
    assert "FAILED" in rendered.splitlines()[0], rendered.splitlines()[0]


def test_the_summary_names_what_failed_outside_the_steps() -> None:
    """Labelling it red is not enough to act on."""
    timing = measure(_events(run_status="failed"))

    rendered = report(timing, command="synthetic", settings=CONFIG.runlog, run_id="r")
    assert "workspace" in rendered
    assert "could not release" in rendered


def test_a_run_that_passed_still_reports_ok() -> None:
    timing = measure(_events(run_status="ok"))

    assert timing.outcome == "ok"
    assert "ok" in report(
        timing, command="synthetic", settings=CONFIG.runlog, run_id="r"
    ).splitlines()[0]


def _recorded_run(root: Path, name: str, events: list[dict], settings) -> Path:
    directory = root / name
    directory.mkdir(parents=True)
    (directory / settings.events).write_text(
        "".join(json.dumps(event) + "\n" for event in events), encoding="utf-8"
    )
    return directory


@pytest.fixture
def recorded(tmp_path, monkeypatch):
    """Two runs: an older one that failed outside its steps, then a good one."""
    settings = CONFIG.runlog
    root = tmp_path / settings.root
    _recorded_run(root, "20260101-000001-aaaaaa-candidate", _events(run_status="failed"), settings)
    _recorded_run(root, "20260101-000002-bbbbbb-candidate", _events(run_status="ok"), settings)
    return tmp_path


def test_run_selection_finds_a_run_that_failed_outside_its_steps(recorded) -> None:
    """`runs last --failed` is how an operator reaches the failure."""
    from capsem_builder.gate.runs import runs as recorded_runs

    settings = CONFIG.model_copy(update={"root": recorded})
    found = recorded_runs(settings)

    assert [directory.name for directory in found] == [
        "20260101-000002-bbbbbb-candidate",
        "20260101-000001-aaaaaa-candidate",
    ]
    failed = [
        directory
        for directory in found
        if measure(
            [json.loads(line) for line in (directory / settings.runlog.events).read_text().splitlines()]
        ).outcome
        == "failed"
    ]
    assert [directory.name for directory in failed] == [
        "20260101-000001-aaaaaa-candidate"
    ]


def test_timing_carries_the_run_level_failures_it_was_told_about() -> None:
    timing = measure(_events(run_status="failed"))

    assert timing.run_failures == {"workspace": "could not release"}
    assert isinstance(timing, Timing)

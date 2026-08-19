"""Citadel guard: the gate a developer waits for has a deadline, and enforces it.

`fast-test` measured twenty-one minutes -- over half of it one un-parallelised
step, and several more minutes the same script run three times. Nothing failed,
because nothing was watching: the timing policy the gate already had compares a
run against its own history, so a run that was always slow is never a
regression. That is the right question for a release and the wrong one for a
name that promises speed.

A budget, and enforced while the run is happening rather than reported
afterwards. A report about last night tells you after you have already waited.
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest
from helpers.gate import RecordingJournal, RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.actions import Call
from capsem.gate.errors import GateError
from capsem.gate.execution import Kind, Speed, step
from capsem.gate.opacity import CallJustification, OpaqueKind, machine_effects
from capsem.gate.plan import Plan

ROOT = Path(__file__).resolve().parents[2]


def _slow_step(label: str, seconds: float):
    return step(
        label,
        Call(
            f"take {seconds}s",
            lambda _context: time.sleep(seconds),
            justification=CallJustification(
                kind=OpaqueKind.PURE_INSPECTION,
                reason="a step that takes measurable time, for a deadline to catch",
                effects=machine_effects(),
            ),
        ),
        kind=Kind.STATIC_TEST,
        speed=Speed.FAST,
    )


def test_a_lane_past_its_deadline_stops_instead_of_finishing(tmp_path: Path) -> None:
    """It fails at the next step boundary, not at the end of the run."""
    from capsem.gate.context import Context

    config = gate_config.load(ROOT)
    plan = Plan("slow")
    first = plan.add(_slow_step("one", 0.2))
    plan.add(_slow_step("two", 0.2), after=(first,))

    context = Context(
        RecordingRunner(ROOT),
        config,
        journal=RecordingJournal(),
        deadline_seconds=0.1,
    )

    try:
        plan.run(context)
    except GateError as error:
        assert "deadline" in str(error).lower(), error
    else:
        raise AssertionError("a plan past its deadline ran to completion")


def test_the_fast_lane_declares_a_budget_and_the_commands_it_bounds() -> None:
    """Named in configuration, so the promise is one value and not a habit."""
    budget = gate_config.load(ROOT).runlog.fast_lane

    assert 0 < budget.seconds <= 600, (
        f"{budget.seconds}s is not a budget a developer would call fast"
    )
    assert budget.commands, "a budget that bounds no command bounds nothing"

    # And that it reaches the lane rather than merely existing. A budget the
    # fast lane is never given is the same as no budget, and reads the same in
    # configuration.
    #
    # `CI` is cleared for this half, because the suite itself runs in CI and the
    # budget is deliberately unenforced there. Asserting the local answer while
    # sitting in the environment that suspends it is how this test passed on my
    # machine and failed the release.
    monkeypatch = pytest.MonkeyPatch()
    monkeypatch.delenv(budget.unenforced_when_set, raising=False)
    try:
        assert budget.for_command("test-fast") == budget.seconds
        assert budget.for_command("candidate") is None, (
            "a release is bounded by patience and the machine lock; failing one "
            "on a stopwatch trades a real proof for a quick one"
        )
    finally:
        monkeypatch.undo()

    # And not in CI, which is a measurement rather than an exemption: a hosted
    # runner starts cold and spent 1285s against this budget, 21m23s of it
    # building the Linux host builder image that a developer's machine has
    # cached. It failed a release the first time it ran. The promise is about
    # the gate somebody sits and waits for.
    enforced = pytest.MonkeyPatch()
    enforced.setenv(budget.unenforced_when_set, "true")
    try:
        assert budget.for_command("test-fast") is None
    finally:
        enforced.undo()
    from capsem.gate.command import GateCommand

    source = Path(GateCommand.__module__.replace(".", "/") + ".py")
    assert "fast_lane.for_command" in (ROOT / "src" / source).read_text(encoding="utf-8"), (
        "nothing hands the budget to the run, so no lane is ever bounded"
    )

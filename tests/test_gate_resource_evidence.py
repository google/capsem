"""Resources run through the same guarded, journaling runner as the plan.

`execute()` builds a `GuardedRunner` for the plan's context, but resources were
constructed from the command's raw runner. Everything a resource does on the
way in or out -- the orphan baseline, Colima, the service launch, the failure
evidence capture -- therefore ran outside the funnel:

* it emitted no `exec` event, so the command that caused a resource failure
  could be absent from the run it failed;
* it did not get the nested-`just`/nested-gate refusal, so the one code path
  that runs *while the machine lock is held* was the one path allowed to start
  a second gate;
* a detached `Launch` recorded nothing at all, because the guard only refused
  re-entry and delegated.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from pathlib import Path

import pytest
from helpers.gate import RecordingJournal, RecordingRunner

from capsem.gate.actions import Run
from capsem.gate.command import GateCommand
from capsem.gate.errors import GateError
from capsem.gate.execution import step
from capsem.gate.lifecycle import Resource
from capsem.gate.plan import Plan
from capsem.gate.proc import Runner

PROJECT_ROOT = Path(__file__).resolve().parents[1]


class _Busy(Resource, name="busy-resource"):
    """A resource that actually runs things, like every real one does."""

    def __init__(self, runner: Runner, *, on_acquire=None) -> None:
        self._runner = runner
        self._on_acquire = on_acquire or ["echo", "acquiring"]

    def acquire(self) -> None:
        self._runner.run(self._on_acquire)

    def release(self) -> None:
        self._runner.run(["echo", "releasing"])

    def preserve(self, error: BaseException) -> None:
        self._runner.run(["echo", "preserving"])


class _Probe(GateCommand, name="resource-evidence-probe", help="a test command"):
    holdings: tuple = ()
    steps: tuple = ()

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return self.holdings(runner) if callable(self.holdings) else self.holdings

    def plan(self) -> Plan:
        plan = Plan(self.name)
        for item in self.steps:
            plan.add(item)
        return plan


@pytest.fixture
def journal(monkeypatch) -> RecordingJournal:
    recording = RecordingJournal()

    @classmethod
    @contextmanager
    def _open(cls, config, command, *, argv=()):
        yield recording

    monkeypatch.setattr("capsem.gate.runlog.RunLog.open", _open)
    monkeypatch.setattr("capsem.gate.recording.RunLog.open", _open)
    return recording


def _probe(runner, **attributes) -> _Probe:
    command = _Probe(runner, argparse.Namespace(dry_run=False, graph=False, timing=False))
    for name, value in attributes.items():
        setattr(command, name, value)
    return command


def test_what_a_resource_runs_is_recorded(journal) -> None:
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, holdings=lambda given: (_Busy(given),))

    command.execute()

    recorded = [" ".join(entry["argv"]) for entry in journal.execs]
    assert "echo acquiring" in recorded
    assert "echo releasing" in recorded


def test_a_resource_may_not_start_a_second_gate(journal) -> None:
    """The one path that runs while the lock is held was the one path allowed
    to ask for it again."""
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=lambda given: (_Busy(given, on_acquire=["just", "test"]),),
    )

    with pytest.raises(GateError, match="starts a second gate"):
        command.execute()


def test_what_a_resource_runs_while_preserving_is_recorded(journal) -> None:
    """`preserve` runs on the failure path, which is when evidence matters."""
    runner = RecordingRunner(PROJECT_ROOT, failures=["false"])
    command = _probe(
        runner,
        holdings=lambda given: (_Busy(given),),
        steps=(step("boom", Run(["false"])),),
    )

    with pytest.raises(GateError):
        command.execute()

    recorded = [" ".join(entry["argv"]) for entry in journal.execs]
    assert "echo preserving" in recorded


def test_a_detached_launch_is_recorded_as_an_execution(journal) -> None:
    """A daemon nobody wrote down is a daemon nobody can account for."""
    runner = RecordingRunner(PROJECT_ROOT)

    class _Daemon(Resource, name="daemon-resource"):
        def __init__(self, given: Runner) -> None:
            self._runner = given

        def acquire(self) -> None:
            self._runner.launch(["capsem-service", "--serve"])

        def release(self) -> None:
            pass

    command = _probe(runner, holdings=lambda given: (_Daemon(given),))

    command.execute()

    assert [" ".join(entry["argv"]) for entry in journal.launches] == [
        "capsem-service --serve"
    ]

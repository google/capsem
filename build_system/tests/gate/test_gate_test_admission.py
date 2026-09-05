"""The complete gate enforces cache-owned test admission before expensive work."""

from __future__ import annotations

import argparse
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.cache.gitmodels import GitImpact
from capsem_builder.cache.models import AdmissionEventKind
from capsem_builder.gate import cli, testadmission
from capsem_builder.gate.candidate import CandidateCommand
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.proc import Runner
from capsem_builder.gate.sourcecommit import SourceCommit
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
COMMIT = SourceCommit("a" * 40)


@pytest.fixture(autouse=True)
def isolate_attempt_history(monkeypatch) -> None:
    monkeypatch.setattr(testadmission.qualificationjournal, "latest_attempt", lambda _: None)


def arguments(**overrides: object) -> argparse.Namespace:
    values = {
        "dry_run": False,
        "graph": False,
        "timing": False,
        "sandbox": None,
        "source_commit": COMMIT,
        "mode": "normal",
        "reason": "",
    }
    values.update(overrides)
    return argparse.Namespace(**values)


def test_candidate_parser_accepts_normal_and_reasoned_force_modes() -> None:
    parser = cli.build_parser()

    normal = parser.parse_args(["candidate", str(COMMIT)])
    forced = parser.parse_args(["candidate", str(COMMIT), "force", "investigate flake"])

    assert normal.mode == "normal"
    assert forced.mode == "force"
    assert forced.reason == "investigate flake"


def test_candidate_delegates_admission_and_completion(monkeypatch) -> None:
    seen: list[tuple[str, SourceCommit | None]] = []
    monkeypatch.setattr(
        "capsem_builder.gate.testadmission.admit",
        lambda command, commit: seen.append(("admit", commit)),
    )
    monkeypatch.setattr(
        "capsem_builder.gate.testadmission.complete",
        lambda command, commit: seen.append(("complete", commit)),
    )
    command = CandidateCommand(Runner(PROJECT_ROOT), arguments())

    command.admit(COMMIT)
    command.completed(COMMIT)

    assert seen == [("admit", COMMIT), ("complete", COMMIT)]


def test_adapter_refuses_low_impact_repeat_with_owned_commands(monkeypatch) -> None:
    baseline = SourceCommit("b" * 40)
    monkeypatch.setattr(testadmission.qualificationevidence, "latest_complete", lambda _: (baseline, object()))
    monkeypatch.setattr(
        testadmission,
        "inspect_git",
        lambda *_: GitImpact(
            baseline=str(baseline),
            target=str(COMMIT),
            ancestor=True,
            commits=2,
            paths=("config/settings/settings.toml",),
        ),
    )
    monkeypatch.setattr(testadmission, "last_admission_event", lambda *_: None)
    command = CandidateCommand(Runner(PROJECT_ROOT), arguments())

    with pytest.raises(GateError, match=r"just focus-test binaries"):
        testadmission.admit(command, COMMIT)


def test_forced_attempt_is_recorded_and_success_resets_only_normal_runs(monkeypatch) -> None:
    recorded = []
    monkeypatch.setattr(testadmission.qualificationevidence, "latest_complete", lambda _: None)
    monkeypatch.setattr(testadmission, "last_admission_event", lambda *_: None)
    monkeypatch.setattr(
        testadmission, "record_admission_event", lambda _root, _path, event: recorded.append(event)
    )
    forced = CandidateCommand(
        Runner(PROJECT_ROOT),
        arguments(mode="force", reason="investigate repeated flake"),
    )
    normal = CandidateCommand(Runner(PROJECT_ROOT), arguments())

    testadmission.admit(forced, COMMIT)
    testadmission.complete(forced, COMMIT)
    testadmission.admit(normal, COMMIT)
    testadmission.complete(normal, COMMIT)

    assert [event.kind for event in recorded] == [
        AdmissionEventKind.FORCED_ATTEMPT,
        AdmissionEventKind.COMPLETE_SUCCESS,
    ]


def test_recording_runner_never_mutates_admission_state(monkeypatch) -> None:
    monkeypatch.setattr(
        testadmission,
        "record_admission_event",
        lambda *_: pytest.fail("an observing runner mutated admission state"),
    )
    command = CandidateCommand(RecordingRunner(PROJECT_ROOT), arguments())

    testadmission.admit(command, COMMIT)
    testadmission.complete(command, COMMIT)


def test_failed_attempt_refuses_before_recording_or_any_plan_work(monkeypatch) -> None:
    from capsem_builder.gate import prefix, qualificationflow
    from capsem_builder.gate.plan import Plan

    monkeypatch.setattr(testadmission.qualificationevidence, "latest_complete", lambda _: None)
    monkeypatch.setattr(
        testadmission.qualificationjournal, "latest_attempt",
        lambda _: SimpleNamespace(end=SimpleNamespace(status="failed")),
    )
    monkeypatch.setattr(testadmission, "last_admission_event", lambda *_: None)
    monkeypatch.setattr(prefix, "active", lambda *_: True)
    monkeypatch.setattr(
        qualificationflow, "decide",
        lambda *args, **kwargs: qualificationflow.Decision(None, None, frozenset(), None),
    )
    command = CandidateCommand(Runner(PROJECT_ROOT), arguments())
    monkeypatch.setattr(command, "_describe", lambda: Plan("candidate"))
    monkeypatch.setattr(command, "reexec", lambda: None)
    monkeypatch.setattr(command, "_recording", lambda **_: pytest.fail("refused run started work"))

    with pytest.raises(GateError, match="explicitly approved retry"):
        command.execute()

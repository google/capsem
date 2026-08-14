from __future__ import annotations

import argparse
import json
import shutil
import subprocess
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import cli, qualificationevidence, qualificationflow
from capsem.gate import config as gate_config
from capsem.gate.candidate import CandidateCommand
from capsem.gate.errors import GateError
from capsem.gate.execution import ResumePolicy, step
from capsem.gate.plan import Plan
from capsem.gate.qualification import LocalQualification
from capsem.gate.qualificationevidence import QualificationPolicy
from capsem.gate.runhistory import read, runs
from capsem.gate.runlog import RunLog
from capsem.gate.runlogschema import (
    FAILED,
    OK,
    SKIPPED,
    PlanShape,
    QualificationComplete,
    QualificationRun,
    RunEnd,
    StepEnd,
)
from capsem.gate.sourcecommit import SourceCommit

PROJECT_ROOT = Path(__file__).resolve().parents[1]
COMMIT = SourceCommit("3" * 40)


@pytest.fixture
def config(tmp_path: Path):
    subprocess.run(["git", "init", "--quiet"], cwd=tmp_path, check=True)
    (tmp_path / "config").mkdir()
    shutil.copy(PROJECT_ROOT / "config/gate.toml", tmp_path / "config/gate.toml")
    return gate_config.load(tmp_path)


def _plan() -> Plan:
    plan = Plan("candidate")
    recorded = plan.add(step("source.record", resume=ResumePolicy.ALWAYS_RUN))
    prepared = plan.add(step("prepare"), after=(recorded,))
    built = plan.add(step("build"), after=(prepared,))
    plan.add(step("source.verify"), after=(built,))
    return plan


def _record_complete(config, monkeypatch: pytest.MonkeyPatch) -> Path:
    plan = _plan()
    monkeypatch.setattr("capsem.gate.runlog.head_revision", lambda _root: str(COMMIT))
    with RunLog.open(config, "candidate", source_commit=str(COMMIT)) as log:
        log.qualification_attempt(COMMIT)
        log.shape(plan.labels, plan.edges)
        for label in plan.labels:
            log.emit(StepEnd(step=label, status=OK, duration_ms=1.0))
        log.emit(
            QualificationComplete(
                source_commit=str(COMMIT),
                source_digest="a" * 64,
                plan_digest=qualificationevidence.plan_digest(plan.labels, plan.edges),
            )
        )
    return qualificationevidence.archive_path(config, COMMIT, log.run_id)


def _candidate(config):
    return CandidateCommand(
        RecordingRunner(config.root),
        argparse.Namespace(
            source_commit=COMMIT,
            dry_run=False,
            graph=False,
            timing=False,
            prefix=None,
            resume_from=None,
            sandbox=None,
        ),
        qualification=LocalQualification(bin_dir=config.modules.default_bin_dir),
        invocation=("capsem-gate", "candidate", str(COMMIT)),
    )


def test_exact_candidate_reuses_completed_journal(config, monkeypatch, capsys) -> None:
    archive = _record_complete(config, monkeypatch)
    monkeypatch.delenv(config.locks.gate.run_marker, raising=False)
    command = _candidate(config)
    monkeypatch.setattr(command, "_describe", _plan)
    monkeypatch.setattr(qualificationflow, "require_local_main", lambda *_args: None)
    monkeypatch.setattr(command, "reexec", lambda *_args: pytest.fail("re-exec reached"))
    monkeypatch.setattr(command, "resources", lambda *_args: pytest.fail("resources reached"))
    monkeypatch.setattr("capsem.gate.command.prefix.active", lambda *_args: pytest.fail("prefix"))

    command.execute()

    assert command._runner.commands == []
    output = capsys.readouterr().out
    assert "already qualified" in output
    assert str(archive) in output
    newest = (config.path(config.runlog.root) / config.runlog.latest_link).resolve()
    events = read(newest, config.runlog)
    reused = [event for event in events if event["event"] == "qualification.reuse"]
    assert len(reused) == 1
    assert reused[0]["qualification"]["run_log"] == str(archive.absolute())
    assert events[-1]["event"] == "run.end"
    assert events[-1]["status"] == OK


def test_exact_candidate_journals_survive_ordinary_run_rotation(
    config, monkeypatch: pytest.MonkeyPatch
) -> None:
    archive = _record_complete(config, monkeypatch)

    assert archive.is_file()
    assert archive.parent.parent.name == config.runlog.source_archive_dir
    assert archive.parent.parent not in runs(config)
    assert qualificationevidence.find_complete(config, COMMIT) is not None


def _write_partial(config, plan: Plan) -> None:
    run_id = "20260814-100000-partial-candidate"
    path = qualificationevidence.archive_path(config, COMMIT, run_id)
    path.parent.mkdir(parents=True, exist_ok=True)
    payloads = [
        {
            "event": "run.start",
            "command": "candidate",
            "argv": ["capsem-gate", "candidate", str(COMMIT)],
            "head": str(COMMIT),
            "source_commit": str(COMMIT),
            "platform": "Linux",
            "machine": "x86_64",
            "cores": 16,
            "free_gb": 100.0,
            "gate_source": "/exact/src/capsem/gate",
            "pycache": "/exact/cache",
        },
        PlanShape(steps=plan.labels, edges=plan.edges).model_dump(),
        StepEnd(step="source.record", status=OK, duration_ms=1).model_dump(),
        StepEnd(step="prepare", status=OK, duration_ms=1).model_dump(),
        StepEnd(step="build", status=FAILED, duration_ms=1).model_dump(),
        StepEnd(step="source.verify", status=SKIPPED, duration_ms=0).model_dump(),
        RunEnd(
            status=FAILED,
            duration_ms=2,
            failures={"build": "boom"},
        ).model_dump(),
    ]
    path.write_text(
        "\n".join(
            json.dumps(
                {
                    "schema": config.runlog.event_schema,
                    "ts": float(index + 1),
                    "run_id": run_id,
                    **payload,
                }
            )
            for index, payload in enumerate(payloads)
        )
        + "\n",
        encoding="utf-8",
    )


def test_partial_exact_evidence_selects_the_retained_prefix_and_frontier(
    config, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    plan = _plan()
    _write_partial(config, plan)
    parent = tmp_path / "prefixes"
    selected = parent / str(COMMIT)
    selected.mkdir(parents=True)
    config = config.model_copy(
        update={"prefix": config.prefix.model_copy(update={"parent": str(parent)})}
    )
    monkeypatch.setattr(qualificationflow, "require_detached_checkout", lambda *_args: None)
    monkeypatch.setattr(qualificationflow, "require_local_main", lambda *_args: None)

    decision = qualificationflow.decide(
        config,
        policy=QualificationPolicy.REUSE_OR_RUN,
        commit=COMMIT,
        plan=plan,
        args=argparse.Namespace(prefix=None, resume_from=None),
        carried=frozenset(),
        reuse_path=None,
    )

    assert decision.resumed is not None
    assert decision.resumed.frontier == "build"
    assert decision.reuse == selected
    assert decision.child_arguments == ("--prefix", str(selected), "--from", "build")
    message = qualificationflow.progress(decision, COMMIT, None)
    assert message is not None
    assert f"resuming {COMMIT} at build from {decision.resumed.parent.run_id}" in message
    assert decision.resumed.parent.run_log in message
    assert decision.resumed.parent.digest in message


@pytest.mark.parametrize("name", ("../escape", "nested/archive", "/absolute"))
def test_archive_configuration_is_one_relative_directory(config, name: str) -> None:
    with pytest.raises(ValueError, match="source_archive_dir"):
        type(config.runlog).model_validate(
            {**config.runlog.model_dump(), "source_archive_dir": name}
        )


def test_archive_configuration_cannot_alias_history_metadata(config) -> None:
    for name in (config.runlog.latest_link, config.runlog.history_lock):
        with pytest.raises(ValueError, match="must not alias"):
            type(config.runlog).model_validate(
                {**config.runlog.model_dump(), "source_archive_dir": name}
            )


def test_qualification_references_reject_path_shaped_run_ids() -> None:
    with pytest.raises(ValueError, match="run_id"):
        QualificationRun(run_id="../other", run_log="/tmp/run.jsonl", digest="a" * 64)


def test_evidence_is_not_consulted_until_the_commit_is_on_main(
    config, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        qualificationflow,
        "require_local_main",
        lambda *_args: (_ for _ in ()).throw(RuntimeError("not on main")),
    )
    with pytest.raises(RuntimeError, match="not on main"):
        qualificationflow.decide(
            config,
            policy=QualificationPolicy.REUSE_OR_RUN,
            commit=COMMIT,
            plan=_plan(),
            args=argparse.Namespace(prefix=None, resume_from=None),
            carried=frozenset(),
            reuse_path=None,
        )


def test_archiving_refuses_a_symlink_directory_before_writing_outside(
    config, tmp_path: Path
) -> None:
    run = config.path(config.runlog.root) / "attempt"
    run.mkdir(parents=True)
    (run / config.runlog.events).write_text("", encoding="utf-8")
    outside = tmp_path / "outside"
    outside.mkdir()
    archive = config.path(config.runlog.root) / config.runlog.source_archive_dir
    archive.symlink_to(outside, target_is_directory=True)

    with pytest.raises(GateError, match="must not be symlinks"):
        qualificationevidence.archive_attempt(config, COMMIT, run)
    assert not (outside / str(COMMIT)).exists()


def test_candidate_cli_has_typed_exact_and_working_tree_modes() -> None:
    parser = cli.build_parser()

    assert parser.parse_args(["candidate", ""]).source_commit is None
    assert parser.parse_args(["candidate", str(COMMIT)]).source_commit == COMMIT
    with pytest.raises(SystemExit):
        parser.parse_args(["candidate", "main"])


def test_manual_exact_continuation_cannot_override_the_journal(
    config, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    plan = _plan()
    _write_partial(config, plan)
    parent = tmp_path / "prefixes"
    selected = parent / str(COMMIT)
    selected.mkdir(parents=True)
    config = config.model_copy(
        update={"prefix": config.prefix.model_copy(update={"parent": str(parent)})}
    )
    monkeypatch.setattr(qualificationflow, "require_local_main", lambda *_args: None)
    monkeypatch.setattr(qualificationflow, "require_detached_checkout", lambda *_args: None)

    for frontier, carried, reuse in (
        ("source.verify", frozenset({"prepare"}), selected),
        ("build", frozenset(), selected),
        ("build", frozenset({"prepare"}), tmp_path / "other"),
    ):
        with pytest.raises(GateError, match="continuation"):
            qualificationflow.decide(
                config,
                policy=QualificationPolicy.REUSE_OR_RUN,
                commit=COMMIT,
                plan=plan,
                args=argparse.Namespace(prefix=str(reuse), resume_from=frontier),
                carried=carried,
                reuse_path=reuse,
            )

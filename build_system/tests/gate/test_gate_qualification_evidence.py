from __future__ import annotations

import json
import shutil
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import qualificationevidence, qualificationjournal
from capsem_builder.gate.execution import ResumePolicy, step
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.runlogschema import (
    CARRIED,
    FAILED,
    OK,
    SKIPPED,
    PlanShape,
    QualificationComplete,
    QualificationResume,
    QualificationReuse,
    RunEnd,
    RunStart,
    StepEnd,
)
from capsem_builder.gate.sourcecommit import SourceCommit

PROJECT_ROOT = Path(__file__).resolve().parents[3]
COMMIT = SourceCommit("1" * 40)
SOURCE_DIGEST = "a" * 64


@pytest.fixture
def config(tmp_path: Path):
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


def _write(config, run_id: str, payloads: list, *, commit: SourceCommit = COMMIT) -> Path:
    target = qualificationevidence.archive_path(config, commit, run_id)
    target.parent.mkdir(parents=True, exist_ok=True)
    lines = []
    for index, payload in enumerate(payloads):
        lines.append(
            json.dumps(
                {
                    "schema": config.runlog.event_schema,
                    "ts": float(index + 1),
                    "run_id": run_id,
                    **payload.model_dump(),
                }
            )
        )
    target.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return target


def _start(commit: SourceCommit = COMMIT) -> RunStart:
    return RunStart(
        command="candidate",
        argv=("capsem-gate", "candidate", str(commit)),
        head=str(commit),
        source_commit=str(commit),
        platform="Linux",
        machine="x86_64",
        cores=16,
        free_gb=100.0,
        gate_source="/exact/build_system/builder/gate",
        pycache="/exact/pycache",
    )


def _end(status: str = OK) -> RunEnd:
    return RunEnd(
        status=status,
        duration_ms=1.0,
        failures={} if status == OK else {"build": "failed"},
    )


def _shape(plan: Plan) -> PlanShape:
    return PlanShape(steps=plan.labels, edges=plan.edges)


def _step(label: str, status: str) -> StepEnd:
    return StepEnd(step=label, status=status, duration_ms=1.0)


def _complete(plan: Plan, *, parent=None) -> QualificationComplete:
    return QualificationComplete(
        source_commit=str(COMMIT),
        source_digest=SOURCE_DIGEST,
        plan_digest=qualificationevidence.plan_digest(plan.labels, plan.edges),
        parent=parent,
    )


def _fresh_success(config, plan: Plan, run_id: str = "20260814-010000-fresh-candidate"):
    path = _write(
        config,
        run_id,
        [
            _start(),
            _shape(plan),
            *(_step(label, OK) for label in plan.labels),
            _complete(plan),
            _end(),
        ],
    )
    return qualificationjournal.reference(config, path)


def test_admission_history_uses_failed_attempt_without_promoting_it_to_proof(config) -> None:
    plan = _plan()
    _fresh_success(config, plan)
    failed = SourceCommit("2" * 40)
    _write(config, "20260814-020000-failed-candidate", [
        _start(failed), _shape(plan), _step("source.record", OK),
        _step("build", FAILED), _end(FAILED),
    ], commit=failed)
    latest = qualificationjournal.latest_attempt(config)
    assert latest is not None and latest.start.source_commit == failed
    assert qualificationevidence.find_complete(config, failed) is None
    complete = qualificationevidence.latest_complete(config)
    assert complete is not None and complete[0] == COMMIT


@pytest.mark.parametrize("ignored", ["live", "empty", "mismatch", "malformed", "symlink"])
def test_admission_history_ignores_non_attempts(config, ignored) -> None:
    plan = _plan()
    older = _fresh_success(config, plan)
    other = SourceCommit("2" * 40)
    payloads = [_start(other), _shape(plan), _step("build", FAILED), _end(FAILED)]
    if ignored == "live":
        payloads.pop()
    elif ignored == "empty":
        payloads.pop(2)
    elif ignored == "mismatch":
        payloads[0] = _start(COMMIT)
    path = _write(config, "20260814-030000-ignored-candidate", payloads, commit=other)
    if ignored == "malformed":
        path.write_text("invalid\n")
    elif ignored == "symlink":
        path.unlink()
        path.symlink_to(older.path)
    latest = qualificationjournal.latest_attempt(config)
    assert latest is not None and latest.reference == older


def test_a_complete_exact_run_log_is_the_reusable_authority(config) -> None:
    plan = _plan()
    expected = _fresh_success(config, plan)

    found = qualificationevidence.find_complete(config, COMMIT)

    assert found is not None
    assert found.reference == expected
    assert found.source_digest == SOURCE_DIGEST
    assert found.reference.run_log == str(expected.path)
    assert len(found.reference.digest) == 64


@pytest.mark.parametrize(
    ("statuses", "terminal"),
    [
        ((OK, OK, FAILED, SKIPPED), FAILED),
        ((OK, CARRIED, OK, OK), OK),
        ((OK, OK, OK, SKIPPED), OK),
    ],
)
def test_failure_carried_or_skipped_without_a_proven_parent_never_qualifies(
    config, statuses: tuple[str, ...], terminal: str
) -> None:
    plan = _plan()
    _write(
        config,
        "20260814-020000-unproven-candidate",
        [
            _start(),
            _shape(plan),
            *(_step(label, status) for label, status in zip(plan.labels, statuses, strict=True)),
            _complete(plan),
            _end(terminal),
        ],
    )

    assert qualificationevidence.find_complete(config, COMMIT) is None


def test_a_fast_success_cannot_recursively_become_qualification(config) -> None:
    plan = _plan()
    original = _fresh_success(config, plan)
    qualificationevidence.archive_path(config, COMMIT, original.run_id).unlink()
    _write(
        config,
        "20260814-030000-reused-candidate",
        [
            _start(),
            PlanShape(steps=("qualification.reuse",), edges=()),
            QualificationReuse(source_commit=str(COMMIT), qualification=original),
            _step("qualification.reuse", OK),
            _end(),
        ],
    )

    assert qualificationevidence.find_complete(config, COMMIT) is None


def test_partial_evidence_selects_the_deepest_proven_resume_frontier(config) -> None:
    plan = _plan()
    parent_path = _write(
        config,
        "20260814-040000-partial-candidate",
        [
            _start(),
            _shape(plan),
            _step("source.record", OK),
            _step("prepare", OK),
            _step("build", FAILED),
            _step("source.verify", SKIPPED),
            _end(FAILED),
        ],
    )

    resumable = qualificationevidence.find_resume(config, COMMIT, plan)

    assert resumable is not None
    assert resumable.frontier == "build"
    assert resumable.carried == frozenset({"prepare"})
    assert resumable.parent == qualificationjournal.reference(config, parent_path)


def test_a_resumed_success_qualifies_only_through_its_exact_parent_log(config) -> None:
    plan = _plan()
    parent_path = _write(
        config,
        "20260814-050000-partial-candidate",
        [
            _start(),
            _shape(plan),
            _step("source.record", OK),
            _step("prepare", OK),
            _step("build", FAILED),
            _step("source.verify", SKIPPED),
            _end(FAILED),
        ],
    )
    parent = qualificationjournal.reference(config, parent_path)
    _write(
        config,
        "20260814-060000-resumed-candidate",
        [
            _start(),
            QualificationResume(
                source_commit=str(COMMIT),
                parent=parent,
                carried_steps=("prepare",),
            ),
            _shape(plan),
            _step("source.record", OK),
            _step("prepare", CARRIED),
            _step("build", OK),
            _step("source.verify", OK),
            _complete(plan, parent=parent),
            _end(),
        ],
    )

    assert qualificationevidence.find_complete(config, COMMIT) is not None

    parent_path.write_text(parent_path.read_text(encoding="utf-8") + "\n", encoding="utf-8")
    assert qualificationevidence.find_complete(config, COMMIT) is None


def test_wrong_commit_plan_or_malformed_event_journal_is_ignored(config) -> None:
    plan = _plan()
    wrong = SourceCommit("2" * 40)
    _write(
        config,
        "20260814-070000-wrong-candidate",
        [
            _start(wrong),
            _shape(plan),
            *(_step(label, OK) for label in plan.labels),
            _complete(plan),
            _end(),
        ],
        commit=COMMIT,
    )
    malformed = qualificationevidence.archive_path(
        config, COMMIT, "20260814-080000-malformed-candidate"
    )
    malformed.parent.mkdir(parents=True, exist_ok=True)
    malformed.write_text("{not json}\n", encoding="utf-8")

    assert qualificationevidence.find_complete(config, COMMIT) is None
    assert qualificationevidence.find_resume(config, COMMIT, plan) is None


def test_reordered_completion_and_duplicate_lineage_events_are_rejected(config) -> None:
    plan = _plan()
    prior = _fresh_success(config, plan, "20260814-090000-prior-candidate")
    reuse = QualificationReuse(source_commit=str(COMMIT), qualification=prior)
    reordered = _write(
        config,
        "20260814-100000-reordered-candidate",
        [_start(), _shape(plan), _complete(plan), *(_step(x, OK) for x in plan.labels), _end()],
    )
    duplicated = _write(
        config,
        "20260814-110000-duplicated-candidate",
        [
            _start(),
            _shape(plan),
            reuse,
            reuse,
            *(_step(label, OK) for label in plan.labels),
            _complete(plan),
            _end(),
        ],
    )

    assert qualificationjournal.load(config, reordered) is None
    assert qualificationjournal.load(config, duplicated) is None

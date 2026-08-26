from __future__ import annotations

import importlib.util
from pathlib import Path
from typing import Any

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "verify_release_recovery_run",
    PROJECT_ROOT / "scripts" / "verify-release-recovery-run.py",
)
assert SPEC is not None and SPEC.loader is not None
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)

SOURCE = "a" * 40
RUN_ID = "42"


def _run() -> dict[str, Any]:
    return {
        "databaseId": 42,
        "headSha": SOURCE,
        "headBranch": f"capsem-source-{SOURCE}",
        "event": "workflow_dispatch",
        "workflowName": "Release",
        "status": "completed",
        "conclusion": "failure",
        "jobs": [
            {"name": "source-identity", "conclusion": "success"},
            {"name": "Complete binary pairing gate", "conclusion": "success"},
            {"name": "create-release", "conclusion": "success"},
            {"name": "assemble-release-channel", "conclusion": "success"},
            {"name": VERIFY.CANDIDATE_JOB, "conclusion": "success"},
            {"name": VERIFY.DEPLOY_JOB, "conclusion": "failure"},
            {"name": VERIFY.POST_DEPLOY_JOB, "conclusion": "skipped"},
        ],
    }


def test_recovery_accepts_only_a_verified_candidate_stopped_at_deployment() -> None:
    VERIFY.verify_recovery_run(_run(), RUN_ID, SOURCE)


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("headSha", "b" * 40),
        ("headBranch", "main"),
        ("event", "push"),
        ("workflowName", "CI"),
        ("status", "in_progress"),
        ("conclusion", "success"),
    ],
)
def test_recovery_rejects_wrong_run_identity(field: str, value: str) -> None:
    run = _run()
    run[field] = value
    with pytest.raises(ValueError, match="identity mismatch"):
        VERIFY.verify_recovery_run(run, RUN_ID, SOURCE)


@pytest.mark.parametrize(
    ("job", "conclusion"),
    [
        (VERIFY.CANDIDATE_JOB, "failure"),
        (VERIFY.DEPLOY_JOB, "success"),
        (VERIFY.POST_DEPLOY_JOB, "success"),
        ("Complete binary pairing gate", "failure"),
    ],
)
def test_recovery_rejects_any_other_job_outcome(job: str, conclusion: str) -> None:
    run = _run()
    for row in run["jobs"]:
        if row["name"] == job:
            row["conclusion"] = conclusion
    with pytest.raises(ValueError, match="did not stop only at deployment"):
        VERIFY.verify_recovery_run(run, RUN_ID, SOURCE)


def test_recovery_rejects_duplicate_jobs() -> None:
    run = _run()
    run["jobs"].append({"name": VERIFY.CANDIDATE_JOB, "conclusion": "success"})
    with pytest.raises(ValueError, match="duplicate job"):
        VERIFY.verify_recovery_run(run, RUN_ID, SOURCE)

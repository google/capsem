"""Prove a failed release run reached verified channel deployment."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

SOURCE_COMMIT = re.compile(r"[0-9a-f]{40}\Z")
ASSEMBLY_JOB = "assemble-release-channel"
CANDIDATE_JOB = "verify-release-candidate"
DEPLOY_JOB = "deploy-release-channel / Publish generated channel"
SKIPPED_DEPLOY_JOB = "deploy-release-channel"
POST_DEPLOY_JOB = "verify-release-downloads"
QUALIFIED_JOBS = (
    "source-identity",
    "runtime-preflight / Materialize (macos-14, arm64)",
    "runtime-preflight / Materialize (ubuntu-24.04, x86_64)",
    "fast-gate / static",
    "Resolve serialized channel source manifest",
    "preflight",
    "build-app-linux (x86_64, ubuntu-24.04)",
    "build-app-linux (arm64, ubuntu-24.04-arm)",
    "build-app-macos",
    "Author binary candidate source manifest",
    "Complete binary pairing gate",
    "Install exact candidate macOS package",
    "Install exact candidate Linux package (x86_64)",
    "Install exact candidate Linux package (arm64)",
    "create-release",
)


def verify_recovery_run(
    run: dict[str, Any],
    run_id: str,
    source_commit: str,
    *,
    failed_job: str = DEPLOY_JOB,
) -> None:
    """Reject recovery unless every edge before channel deployment passed."""
    if failed_job not in {ASSEMBLY_JOB, DEPLOY_JOB}:
        raise ValueError(f"unsupported recovery failure job: {failed_job}")
    if SOURCE_COMMIT.fullmatch(source_commit) is None:
        raise ValueError("source commit must be 40-character lowercase hexadecimal")
    expected = {
        "databaseId": int(run_id),
        "headSha": source_commit,
        "headBranch": f"capsem-source-{source_commit}",
        "event": "workflow_dispatch",
        "workflowName": "Release",
        "status": "completed",
        "conclusion": "failure",
    }
    mismatches = {
        field: {"expected": value, "actual": run.get(field)}
        for field, value in expected.items()
        if run.get(field) != value
    }
    if mismatches:
        raise ValueError(f"release recovery run identity mismatch: {mismatches}")

    jobs = run.get("jobs")
    if not isinstance(jobs, list):
        raise ValueError("release recovery run has no job list")
    conclusions: dict[str, str] = {}
    for job in jobs:
        if not isinstance(job, dict):
            raise ValueError("release recovery run contains a malformed job")
        name = job.get("name")
        conclusion = job.get("conclusion")
        if not isinstance(name, str) or not isinstance(conclusion, str):
            raise ValueError(f"release recovery run contains a malformed job: {job}")
        if name in conclusions:
            raise ValueError(f"release recovery run contains duplicate job {name!r}")
        conclusions[name] = conclusion

    required = dict.fromkeys(QUALIFIED_JOBS, "success")
    if failed_job == ASSEMBLY_JOB:
        required.update(
            {
                ASSEMBLY_JOB: "failure",
                CANDIDATE_JOB: "skipped",
                SKIPPED_DEPLOY_JOB: "skipped",
                POST_DEPLOY_JOB: "skipped",
            }
        )
    else:
        required.update(
            {
                ASSEMBLY_JOB: "success",
                CANDIDATE_JOB: "success",
                DEPLOY_JOB: "failure",
                POST_DEPLOY_JOB: "skipped",
            }
        )
    failures = {
        name: {"expected": conclusion, "actual": conclusions.get(name)}
        for name, conclusion in required.items()
        if conclusions.get(name) != conclusion
    }
    unexpected = {
        name: conclusion
        for name, conclusion in conclusions.items()
        if name not in required and conclusion != "success"
    }
    if failures or unexpected:
        stage = "deployment" if failed_job == DEPLOY_JOB else failed_job
        raise ValueError(
            f"release recovery run did not stop only at {stage}: "
            f"required={failures}, unexpected={unexpected}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--failed-job", choices=(ASSEMBLY_JOB, DEPLOY_JOB), default=DEPLOY_JOB)
    args = parser.parse_args()
    try:
        run = json.loads(args.run.read_text(encoding="utf-8"))
        if not isinstance(run, dict):
            raise ValueError("release recovery run JSON must be an object")
        verify_recovery_run(
            run,
            args.run_id,
            args.source_commit,
            failed_job=args.failed_job,
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"release recovery refused: {error}", file=sys.stderr)
        return 1
    print(f"release run {args.run_id} may resume verified channel deployment")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Fail closed unless an exact commit passed remote release qualification."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from typing import NamedTuple


WORKFLOW = "release-qualification.yaml"
RUN_FIELDS = "databaseId,displayTitle,status,conclusion,headSha,url"
FULL_SHA = re.compile(r"[0-9a-f]{40}")


class QualificationResult(NamedTuple):
    ok: bool
    detail: str
    run_id: int | None = None
    url: str | None = None


def check_release_qualification(repo: str, sha: str) -> QualificationResult:
    """Return the successful qualification for *sha*, never a nearby run."""
    if FULL_SHA.fullmatch(sha) is None:
        return QualificationResult(
            False,
            f"expected a 40-character lowercase commit SHA, got {sha!r}",
        )

    command = [
        "gh",
        "run",
        "list",
        "--repo",
        repo,
        "--workflow",
        WORKFLOW,
        "--commit",
        sha,
        "--event",
        "workflow_dispatch",
        "--limit",
        "100",
        "--json",
        RUN_FIELDS,
    ]
    completed = subprocess.run(command, text=True, capture_output=True, check=False)
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "gh run list failed"
        return QualificationResult(False, detail)

    try:
        runs = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        return QualificationResult(False, f"invalid gh run JSON: {error}")
    if not isinstance(runs, list):
        return QualificationResult(False, "gh run list did not return a JSON array")

    expected_title = f"Qualify release {sha}"
    for run in runs:
        if not isinstance(run, dict):
            continue
        if (
            run.get("headSha") == sha
            and run.get("displayTitle") == expected_title
            and run.get("status") == "completed"
            and run.get("conclusion") == "success"
            and isinstance(run.get("databaseId"), int)
            and isinstance(run.get("url"), str)
        ):
            return QualificationResult(
                True,
                f"exact SHA {sha} passed remote qualification",
                run["databaseId"],
                run["url"],
            )

    observed = ", ".join(
        f"{run.get('headSha', '?')}:{run.get('status', '?')}/{run.get('conclusion', '?')}"
        for run in runs
        if isinstance(run, dict)
    )
    suffix = f"; observed {observed}" if observed else ""
    return QualificationResult(
        False,
        f"no successful completed qualification for exact SHA {sha}{suffix}",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="google/capsem")
    parser.add_argument("--sha", required=True)
    args = parser.parse_args()

    result = check_release_qualification(args.repo, args.sha)
    if not result.ok:
        print(f"release qualification rejected: {result.detail}", file=sys.stderr)
        return 1
    print(f"release qualification accepted: {result.detail}")
    print(f"run: {result.url}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

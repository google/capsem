"""Read-only, paginated GitHub collection with retained historical attempt evidence."""

from __future__ import annotations

import datetime as dt
import json
import os
import re
import subprocess
from collections.abc import Callable

FAILURES = {"failure", "timed_out", "startup_failure", "action_required", "stale"}


class GitHub:
    """gh owns authentication and safe authenticated cross-host redirect handling."""

    def __init__(self, fetch: Callable | None = None):
        self.fetch = fetch or self.get

    @staticmethod
    def get(path: str):
        result = subprocess.run(
            ["gh", "api", path],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        if result.returncode:
            raise RuntimeError(f"GitHub request failed for {path}: {result.stderr[:400]}")
        return json.loads(result.stdout)

    def pages(self, path: str, key: str | None = None) -> list:
        result = []
        for page in range(1, 101):
            value = self.fetch(f"{path}{'&' if '?' in path else '?'}per_page=100&page={page}")
            rows = value[key] if key else value
            if not isinstance(rows, list):
                raise ValueError(f"Expected a paginated list from {path}")
            result.extend(rows)
            if len(rows) < 100:
                return result
        raise RuntimeError(f"GitHub pagination exceeded 100 pages for {path}")


def update_attempts(cached: list[dict], repo: str, client: GitHub) -> list[dict]:
    """Retain expired/deleted historical runs; only fetch details for unseen attempts."""
    attempts = {(str(r["run_id"]), int(r["attempt"])): r for r in cached}
    for workflow, lane in [("release.yaml", "binary"), ("release-assets.yaml", "runtime")]:
        runs = client.pages(f"repos/{repo}/actions/workflows/{workflow}/runs", "workflow_runs")
        for run in runs:
            for attempt in range(1, run.get("run_attempt", 1) + 1):
                key = (str(run["id"]), attempt)
                if key in attempts and attempts[key].get("status") == "completed":
                    continue
                detail = (
                    run
                    if attempt == run.get("run_attempt", 1)
                    else client.fetch(f"repos/{repo}/actions/runs/{run['id']}/attempts/{attempt}")
                )
                jobs = client.pages(
                    f"repos/{repo}/actions/runs/{run['id']}/attempts/{attempt}/jobs", "jobs"
                )
                end = max(
                    (j["completed_at"] for j in jobs if j.get("completed_at")),
                    default=detail["updated_at"],
                )
                start = detail.get("run_started_at") or detail["created_at"]
                attempts[key] = {
                    "run_id": str(run["id"]),
                    "attempt": attempt,
                    "lane": lane,
                    "status": detail["status"],
                    "conclusion": detail.get("conclusion") or "",
                    "started_at_utc": start,
                    "completed_at_utc": end,
                    "duration_seconds": _seconds(start, end),
                    "summed_job_seconds": sum(
                        _seconds(j.get("started_at"), j.get("completed_at")) for j in jobs
                    ),
                    "source_commit": detail["head_sha"],
                    "version": (
                        match[0]
                        if (match := re.search(r"v\d+\.\d+\.\d+", detail.get("display_title", "")))
                        else ""
                    ),
                    "run_url": f"https://github.com/{repo}/actions/runs/{run['id']}/attempts/{attempt}",
                }
    return sorted(
        attempts.values(), key=lambda r: (r["started_at_utc"], str(r["run_id"]), int(r["attempt"]))
    )


def _seconds(start: str | None, end: str | None) -> float:
    if not start or not end:
        return 0
    return max(
        0,
        (
            dt.datetime.fromisoformat(end.replace("Z", "+00:00"))
            - dt.datetime.fromisoformat(start.replace("Z", "+00:00"))
        ).total_seconds(),
    )


def monthly_outcomes(attempts: list[dict]) -> list[dict]:
    if not attempts:
        return []
    start = min(r["started_at_utc"][:10] for r in attempts)
    last = max(r["started_at_utc"][:7] for r in attempts)
    cursor = dt.date.fromisoformat(start).replace(day=1)
    result = []
    while cursor.strftime("%Y-%m") <= last:
        month = cursor.strftime("%Y-%m")
        rows = [r for r in attempts if r["started_at_utc"].startswith(month)]
        result.append(
            {
                "month": month,
                "success": sum(r["conclusion"] == "success" for r in rows),
                "failure": sum(r["conclusion"] in FAILURES for r in rows),
                "cancelled": sum(r["conclusion"] == "cancelled" for r in rows),
                "failedJobHours": round(
                    sum(
                        float(r.get("summed_job_seconds") or 0)
                        for r in rows
                        if r["conclusion"] in FAILURES
                    )
                    / 3600,
                    3,
                ),
            }
        )
        cursor = (cursor.replace(day=28) + dt.timedelta(days=4)).replace(day=1)
    return result


def authenticated() -> bool:
    return bool(os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN"))


def publication_timings(publications: list[dict], attempts: list[dict]) -> list[dict]:
    result = []
    for release in sorted(publications, key=lambda r: r.get("published_at") or ""):
        if release.get("draft") or not re.fullmatch(r"v\d+\.\d+\.\d+", release["tag_name"]):
            continue
        published = release.get("published_at")
        candidates = [
            r
            for r in attempts
            if r.get("version") == release["tag_name"]
            and r["started_at_utc"] <= published <= r.get("completed_at_utc", "")
        ]
        producing = max(candidates, key=lambda r: r["started_at_utc"]) if candidates else None
        result.append(
            {
                "version": release["tag_name"],
                "publishedAt": published,
                "minutes": round(float(producing["duration_seconds"]) / 60, 2)
                if producing
                else None,
                "conclusion": producing["conclusion"] if producing else None,
                "url": release["html_url"],
            }
        )
    return result


def success_intervals(attempts: list[dict]) -> list[dict]:
    """Failures since the prior successful completion, independently per release lane."""
    pending: dict[str, dict] = {}
    result = []
    for row in sorted(attempts, key=lambda r: r.get("completed_at_utc") or r["started_at_utc"]):
        lane = row["lane"]
        counts = pending.setdefault(lane, {"failures": 0, "cancelled": 0})
        conclusion = row["conclusion"]
        if conclusion in FAILURES:
            counts["failures"] += 1
        elif conclusion == "cancelled":
            counts["cancelled"] += 1
        elif conclusion == "success":
            result.append(
                {
                    "label": f"{row['completed_at_utc'][:10]} {lane} {row.get('version') or row['run_id']} #{row['attempt']}",
                    "lane": lane,
                    **counts,
                    "minutes": round(float(row["duration_seconds"]) / 60, 2),
                    "url": row["run_url"],
                }
            )
            pending[lane] = {"failures": 0, "cancelled": 0}
    return result

"""Sample the actual main lineage, including explicitly restored pre-reset history."""

from __future__ import annotations

import calendar
import datetime as dt
import subprocess
from pathlib import Path


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True, timeout=60).strip()


def _chain(repo: Path, ref: str) -> list[tuple[str, dt.datetime]]:
    return [
        (sha, dt.datetime.fromisoformat(date).astimezone(dt.UTC))
        for sha, date in (
            row.split(" ", 1)
            for row in git(
                repo, "log", "--first-parent", "--reverse", "--format=%H %cI", ref
            ).splitlines()
        )
    ]


def monthly_snapshots(repo: Path, ref: str = "HEAD") -> list[dict]:
    """Bridge only a merge explicitly restoring history, never arbitrary dated branches."""
    chain = _chain(repo, ref)
    root = chain[0][0]
    merges = git(repo, "log", "--merges", "--format=%P%x09%s", ref).splitlines()
    for row in reversed(merges):
        parents, subject = row.split("\t", 1)
        parents = parents.split()
        if len(parents) != 2 or parents[0] != root:
            continue
        if "restore" not in subject.lower() or "history" not in subject.lower():
            continue
        old = _chain(repo, parents[1])
        if old[0][0] != root:
            chain = old + chain
            break
    first, last = chain[0][1].date(), chain[-1][1].date()
    days = [(first, "initial_commit")]
    cursor = first.replace(day=1)
    while cursor <= last:
        end = cursor.replace(day=calendar.monthrange(cursor.year, cursor.month)[1])
        day = min(end, last)
        if day != first:
            days.append((day, "month_end" if day == end else "latest_partial_month"))
        cursor = (cursor.replace(day=28) + dt.timedelta(days=4)).replace(day=1)
    result = []
    for day, kind in days:
        cutoff = dt.datetime.combine(day + dt.timedelta(days=1), dt.time.min, dt.UTC)
        candidates = [row for row in chain if row[1] < cutoff]
        sha, date = chain[0] if kind == "initial_commit" else candidates[-1]
        result.append(
            {
                "date": day.isoformat(),
                "month": day.strftime("%Y-%m"),
                "commit": sha,
                "committedAt": date.isoformat(),
                "kind": kind,
            }
        )
    return result

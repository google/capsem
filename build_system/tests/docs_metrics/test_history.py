"""Exercise monthly sampling against real Git DAGs, including restored main history."""

import os
import subprocess
from pathlib import Path

from capsem_builder.docs_metrics.history import monthly_snapshots


def git(repo: Path, *args: str, date: str = "2025-10-01T12:00:00Z") -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        check=True,
        capture_output=True,
        text=True,
        env={**os.environ, "GIT_AUTHOR_DATE": date, "GIT_COMMITTER_DATE": date},
        timeout=10,
    )
    return result.stdout.strip()


def commit(repo: Path, name: str, date: str) -> str:
    (repo / "source.py").write_text(name)
    git(repo, "add", "source.py")
    git(repo, "commit", "-qm", name, date=date)
    return git(repo, "rev-parse", "HEAD")


def repository(tmp_path: Path) -> Path:
    git(tmp_path, "init", "-qb", "main")
    git(tmp_path, "config", "user.name", "Fixture")
    git(tmp_path, "config", "user.email", "fixture@example.test")
    return tmp_path


def test_months_include_initial_commit_and_do_not_select_feature_work(tmp_path):
    repo = repository(tmp_path)
    initial = commit(repo, "initial", "2025-10-01T12:00:00Z")
    git(repo, "checkout", "-qb", "feature")
    commit(repo, "unmerged feature", "2025-11-29T12:00:00Z")
    git(repo, "checkout", "main")
    latest = commit(repo, "main update", "2025-12-15T12:00:00Z")
    rows = monthly_snapshots(repo)
    assert [(r["date"], r["commit"]) for r in rows] == [
        ("2025-10-01", initial),
        ("2025-10-31", initial),
        ("2025-11-30", initial),
        ("2025-12-15", latest),
    ]
    assert rows[0]["kind"] == "initial_commit"
    assert rows[-1]["kind"] == "latest_partial_month"


def test_explicit_history_restore_recovers_old_lineage(tmp_path):
    repo = repository(tmp_path)
    initial = commit(repo, "initial", "2025-10-01T12:00:00Z")
    old_tip = commit(repo, "old main", "2026-05-20T12:00:00Z")
    git(repo, "branch", "old-main")
    git(repo, "checkout", "--orphan", "new-main")
    reset = commit(repo, "reset snapshot", "2026-06-06T12:00:00Z")
    git(
        repo,
        "merge",
        "--allow-unrelated-histories",
        "-s",
        "ours",
        "--no-ff",
        "old-main",
        "-m",
        "restore main history",
        date="2026-06-07T12:00:00Z",
    )
    latest = commit(repo, "current", "2026-07-10T12:00:00Z")
    rows = monthly_snapshots(repo)
    assert rows[0]["commit"] == initial
    assert next(r for r in rows if r["date"] == "2026-05-31")["commit"] == old_tip
    assert rows[-1]["commit"] == latest
    assert reset not in [r["commit"] for r in rows]


def test_initial_month_end_has_one_point(tmp_path):
    repo = repository(tmp_path)
    commit(repo, "initial", "2025-10-31T23:59:00Z")
    rows = monthly_snapshots(repo)
    assert len(rows) == 1
    assert rows[0]["date"] == "2025-10-31"

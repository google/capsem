"""Behavioral tests for exact-SHA release qualification verification."""

from __future__ import annotations

import importlib.util
import json
import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "check-release-qualification.py"
SHA = "a" * 40


def _load_module():
    spec = importlib.util.spec_from_file_location("check_release_qualification", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _run(*, sha: str = SHA, status: str = "completed", conclusion: str = "success"):
    return {
        "databaseId": 123,
        "displayTitle": f"Qualify release {sha}",
        "status": status,
        "conclusion": conclusion,
        "headSha": sha,
        "url": "https://github.example/runs/123",
    }


def test_accepts_only_a_completed_success_for_the_exact_sha(monkeypatch) -> None:
    module = _load_module()
    captured: list[list[str]] = []

    def fake_run(command, **_kwargs):
        captured.append(command)
        return subprocess.CompletedProcess(command, 0, json.dumps([_run()]), "")

    monkeypatch.setattr(module.subprocess, "run", fake_run)

    result = module.check_release_qualification("google/capsem", SHA)

    assert result.ok is True
    assert result.run_id == 123
    assert result.url == "https://github.example/runs/123"
    assert captured == [
        [
            "gh",
            "run",
            "list",
            "--repo",
            "google/capsem",
            "--workflow",
            "release-qualification.yaml",
            "--commit",
            SHA,
            "--event",
            "workflow_dispatch",
            "--limit",
            "100",
            "--json",
            "databaseId,displayTitle,status,conclusion,headSha,url",
        ]
    ]


def test_rejects_successful_run_for_a_different_sha(monkeypatch) -> None:
    module = _load_module()
    other_sha = "b" * 40
    monkeypatch.setattr(
        module.subprocess,
        "run",
        lambda command, **_kwargs: subprocess.CompletedProcess(
            command, 0, json.dumps([_run(sha=other_sha)]), ""
        ),
    )

    result = module.check_release_qualification("google/capsem", SHA)

    assert result.ok is False
    assert "no successful completed qualification for exact SHA" in result.detail


def test_rejects_pending_failed_and_malformed_runs(monkeypatch) -> None:
    module = _load_module()
    runs = [
        _run(status="in_progress", conclusion=""),
        _run(conclusion="failure"),
        {"headSha": SHA, "status": "completed", "conclusion": "success"},
    ]
    monkeypatch.setattr(
        module.subprocess,
        "run",
        lambda command, **_kwargs: subprocess.CompletedProcess(
            command, 0, json.dumps(runs), ""
        ),
    )

    result = module.check_release_qualification("google/capsem", SHA)

    assert result.ok is False
    assert "no successful completed qualification for exact SHA" in result.detail


def test_rejects_non_full_commit_sha_without_querying_github(monkeypatch) -> None:
    module = _load_module()
    monkeypatch.setattr(
        module.subprocess,
        "run",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("must not query")),
    )

    result = module.check_release_qualification("google/capsem", "deadbeef")

    assert result.ok is False
    assert "40-character lowercase commit SHA" in result.detail

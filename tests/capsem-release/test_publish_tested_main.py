from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import sys

import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "publish-tested-main.py"
SPEC = importlib.util.spec_from_file_location("publish_tested_main", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
PUBLISH = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = PUBLISH
SPEC.loader.exec_module(PUBLISH)


def _git(root: Path, *args: str) -> str:
    return subprocess.run(
        ("git", *args),
        cwd=root,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()


def _repository(tmp_path: Path) -> tuple[Path, Path]:
    remote = tmp_path / "remote.git"
    work = tmp_path / "work"
    _git(tmp_path, "init", "--bare", str(remote))
    _git(tmp_path, "init", "-b", "main", str(work))
    _git(work, "config", "user.name", "Release Test")
    _git(work, "config", "user.email", "release@example.test")
    _git(work, "config", "commit.gpgsign", "false")
    (work / "tracked.txt").write_text("initial\n", encoding="utf-8")
    _git(work, "add", "tracked.txt")
    _git(work, "commit", "-m", "initial")
    _git(work, "remote", "add", "origin", str(remote))
    _git(work, "push", "-u", "origin", "main")
    return work, remote


def test_clean_tested_ahead_commit_is_fast_forwarded(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    work, remote = _repository(tmp_path)
    (work / "tracked.txt").write_text("tested\n", encoding="utf-8")
    _git(work, "commit", "-am", "tested")
    expected = _git(work, "rev-parse", "HEAD")
    monkeypatch.setattr(PUBLISH, "ROOT", work)

    PUBLISH.publish_tested_main(expected)

    assert _git(remote, "rev-parse", "main") == expected


def test_changed_head_or_dirty_tree_cannot_be_published(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    work, remote = _repository(tmp_path)
    expected = _git(work, "rev-parse", "HEAD")
    monkeypatch.setattr(PUBLISH, "ROOT", work)
    (work / "tracked.txt").write_text("new commit\n", encoding="utf-8")
    _git(work, "commit", "-am", "new commit")

    with pytest.raises(RuntimeError, match="HEAD changed"):
        PUBLISH.publish_tested_main(expected)
    assert _git(remote, "rev-parse", "main") == expected

    current = _git(work, "rev-parse", "HEAD")
    (work / "tracked.txt").write_text("dirty\n", encoding="utf-8")
    with pytest.raises(RuntimeError, match="source tree to be clean"):
        PUBLISH.publish_tested_main(current)
    assert _git(remote, "rev-parse", "main") == expected


def test_diverged_tested_main_is_never_pushed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    work, remote = _repository(tmp_path)
    other = tmp_path / "other"
    _git(tmp_path, "clone", str(remote), str(other))
    _git(other, "config", "user.name", "Remote Test")
    _git(other, "config", "user.email", "remote@example.test")
    _git(other, "config", "commit.gpgsign", "false")
    (other / "remote.txt").write_text("remote\n", encoding="utf-8")
    _git(other, "add", "remote.txt")
    _git(other, "commit", "-m", "remote")
    _git(other, "push", "origin", "main")
    remote_head = _git(other, "rev-parse", "HEAD")

    (work / "tracked.txt").write_text("local\n", encoding="utf-8")
    _git(work, "commit", "-am", "local")
    expected = _git(work, "rev-parse", "HEAD")
    monkeypatch.setattr(PUBLISH, "ROOT", work)

    with pytest.raises(RuntimeError, match="diverged"):
        PUBLISH.publish_tested_main(expected)

    assert _git(remote, "rev-parse", "main") == remote_head

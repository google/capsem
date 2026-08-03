"""Docker builds retain Git identity when the gate runs from a linked worktree."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate.errors import GateError
from capsem.gate.gitmetadata import docker_git_metadata_mount
from capsem.gate.proc import Runner


def _git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
    )


def _linked_worktree(tmp_path: Path) -> tuple[Path, Path]:
    repository = tmp_path / "repository"
    repository.mkdir()
    _git("init", "--quiet", cwd=repository)
    _git(
        "-c",
        "user.name=Capsem Test",
        "-c",
        "user.email=test@capsem.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "fixture",
        cwd=repository,
    )
    worktree = tmp_path / "linked"
    _git("worktree", "add", "--quiet", "--detach", str(worktree), cwd=repository)
    return repository, worktree


def test_linked_worktree_mounts_its_external_git_metadata_read_only(tmp_path: Path) -> None:
    repository, worktree = _linked_worktree(tmp_path)

    mount = docker_git_metadata_mount(Runner(worktree))

    common = (repository / ".git").resolve()
    assert mount == ("-v", f"{common}:{common}:ro")


def test_ordinary_checkout_needs_no_second_git_mount(tmp_path: Path) -> None:
    repository, _ = _linked_worktree(tmp_path)

    assert docker_git_metadata_mount(Runner(repository)) == ()


def test_linked_worktree_fails_closed_when_git_cannot_resolve_metadata(
    tmp_path: Path,
) -> None:
    root = tmp_path / "broken"
    root.mkdir()
    (root / ".git").write_text("gitdir: /missing/git/metadata\n", encoding="utf-8")

    with pytest.raises(GateError, match="linked worktree Git metadata"):
        docker_git_metadata_mount(RecordingRunner(root))

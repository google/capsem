"""A private inspection repository copies reachable history, not Git garbage."""

import subprocess
from pathlib import Path

from capsem_builder.gate.snapshot import _materialize_repository


def test_private_repository_excludes_unreachable_objects_and_survives_source_removal(
    tmp_path: Path,
) -> None:
    source = tmp_path / "source"
    source.mkdir()

    def git(*args: str, cwd: Path = source, data: str | None = None):
        return subprocess.run(
            ["git", *args], cwd=cwd, input=data, capture_output=True, text=True,
            check=True, timeout=15,
        ).stdout.strip()

    git("init", "-b", "main")
    git("config", "commit.gpgsign", "false")
    git("-c", "user.name=Test", "-c", "user.email=test@example.com",
        "commit", "--allow-empty", "-m", "reachable history")
    head = git("rev-parse", "HEAD")
    unused = git("hash-object", "-w", "--stdin", data="unreachable generated artifact\n")
    target = tmp_path / "target"
    target.mkdir()
    _materialize_repository(source, target)
    assert git("rev-parse", "HEAD", cwd=target) == head
    assert not (target / ".git/objects/info/alternates").exists()
    missing = subprocess.run(
        ["git", "cat-file", "-e", unused], cwd=target, capture_output=True, timeout=15,
    )
    assert missing.returncode != 0, (
        "inspection snapshots must not copy unreachable artifacts or interrupted packs; "
        "those bytes turned source-only Tart contracts into multi-gigabyte clones"
    )
    (source / ".git").rename(source / "retired-git")
    assert git("cat-file", "-t", head, cwd=target) == "commit"

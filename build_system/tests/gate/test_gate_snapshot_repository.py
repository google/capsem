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


def test_private_repository_index_is_already_current_for_the_copied_tree(tmp_path: Path) -> None:
    """The prefix's first `git diff` must not pay to hash every tracked file.

    An index filled without stat data made the first comparison inside a
    prefix rehash the whole tree, and it landed on a benchmark's five-second
    `git diff HEAD` provenance stamp under load (functional lane, 2026-09-25).
    """
    source = tmp_path / "source"
    source.mkdir()
    for name in ("a.txt", "b.txt"):
        (source / name).write_text(f"{name}\n", encoding="utf-8")

    def run(*args: str, cwd: Path = source) -> str:
        return subprocess.run(
            ["git", *args], cwd=cwd, capture_output=True, text=True, check=True, timeout=15,
        ).stdout

    run("init", "-b", "main")
    run("add", ".")
    run("-c", "user.name=Test", "-c", "user.email=test@example.com",
        "-c", "commit.gpgsign=false", "commit", "-m", "tree")
    target = tmp_path / "target"
    target.mkdir()
    for name in ("a.txt", "b.txt"):
        (target / name).write_text((source / name).read_text(encoding="utf-8"), encoding="utf-8")
    (target / "b.txt").write_text("changed\n", encoding="utf-8")

    _materialize_repository(source, target)

    # `diff-files` trusts the index's stat data and never refreshes it, so a
    # file it names is one the next `git diff` would have to hash.
    assert run("diff-files", "--name-only", cwd=target).split() == ["b.txt"]
    assert run("ls-files", cwd=target).split() == ["a.txt", "b.txt"]

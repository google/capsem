"""A release refuses a working tree it is not going to publish.

The run works from a detached copy of the selected commit, so an uncommitted
change is not in the release and cannot be. That is correct and it is silent:
three times in one afternoon a fix was written, verified by hand, and then
released without being committed. The gate built the old bytes, the change
appeared to have done nothing, and the next hour went into explaining a result
that had been right all along.

`--force` exists for a deliberate difference and has to be typed.
"""

from __future__ import annotations

import argparse
import importlib.util
import subprocess
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate.command import GateCommand
from capsem.gate.sourcecommit import SourceCommit

importlib.import_module("capsem.gate.cli")  # registers every command

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = PROJECT_ROOT / "scripts" / "require-clean-worktree.py"
COMMIT = SourceCommit("0" * 40)

_SPEC = importlib.util.spec_from_file_location("require_clean_worktree", SCRIPT)
assert _SPEC is not None and _SPEC.loader is not None
CHECK = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(CHECK)

RELEASES = {
    "release-binaries": {"channel": "stable"},
    "release-profile": {"channel": "stable", "profile": "code"},
}


def _plan(name: str, *, force: str):
    parsed = argparse.Namespace(
        dry_run=False,
        graph=False,
        timing=False,
        force=force,
        source_commit=COMMIT,
        **RELEASES[name],
    )
    return GateCommand.registry[name](RecordingRunner(PROJECT_ROOT), parsed).plan()


@pytest.fixture
def repository(tmp_path: Path) -> Path:
    root = tmp_path / "checkout"
    root.mkdir()
    for argv in (
        ("git", "init", "-q", "."),
        ("git", "config", "user.email", "gate@capsem.test"),
        ("git", "config", "user.name", "gate"),
    ):
        subprocess.run(argv, cwd=root, check=True, capture_output=True)
    (root / "tracked.txt").write_text("committed\n", encoding="utf-8")
    subprocess.run(("git", "add", "tracked.txt"), cwd=root, check=True, capture_output=True)
    subprocess.run(("git", "commit", "-qm", "init"), cwd=root, check=True, capture_output=True)
    return root


def test_a_clean_tree_releases(repository: Path) -> None:
    assert CHECK.dirty_paths(repository) == []
    assert CHECK.main([str(repository), str(COMMIT)]) == 0


@pytest.mark.parametrize(
    ("name", "write"),
    (
        ("an untracked file", lambda root: (root / "scratch.txt").write_text("x")),
        ("a modified file", lambda root: (root / "tracked.txt").write_text("changed")),
    ),
)
def test_any_uncommitted_change_refuses(repository: Path, name: str, write) -> None:
    """Untracked counts too: a new file is the shape this keeps catching."""
    write(repository)

    with pytest.raises(SystemExit) as refusal:
        CHECK.main([str(repository), str(COMMIT)])

    message = str(refusal.value)
    assert "uncommitted change" in message, name
    # Both remedies, because the right one depends on what the operator meant.
    assert "Commit them" in message
    assert "--force" in message


def test_the_refusal_names_what_is_dirty(repository: Path) -> None:
    """A count alone sends the reader back to `git status` to learn anything."""
    (repository / "scratch.txt").write_text("x")

    with pytest.raises(SystemExit) as refusal:
        CHECK.main([str(repository), str(COMMIT)])

    assert "scratch.txt" in str(refusal.value)


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_the_check_gates_qualification_and_force_removes_it(name: str) -> None:
    """It is first, and everything that publishes waits behind it."""
    guarded = list(_plan(name, force="false").labels)
    forced = list(_plan(name, force="true").labels)

    assert guarded[0] == "source.worktree-clean"
    assert guarded.index("source.worktree-clean") < guarded.index("qualification.accept")
    assert guarded.index("source.worktree-clean") < guarded.index("source.publish-ref")

    assert "source.worktree-clean" not in forced
    assert len(forced) == len(guarded) - 1

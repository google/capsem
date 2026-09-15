"""Filesystem primitives, and the two that carry a safety property.

Most of these are one call wrapped so it can be rendered and timed. Two are
not, and both encode a defect this project has already met.

`AtomicReplace` exists because the initrd is a hash-named hardlink shared with
every asset tree built from the same bytes. Truncating it in place corrupts all
of them, which is why `_pack-initrd` wrote `${INITRD}.tmp.$$` and moved it
(justfile:1470-1472) -- a rule that lived in one recipe and nothing enforced.

`Symlink` exists because `cache/target/assets/current` is repointed by whichever image
builder finished last, so the host-architecture VM proof that follows needs it
aimed deliberately and then checked (assets.py:107-115).
"""

from __future__ import annotations

import os
from pathlib import Path

import blake3
import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.fileactions import (
    AtomicReplace,
    Copy,
    Hash,
    MakeDir,
    Remove,
    RequireFile,
    RequireNonEmpty,
    Symlink,
    digest_of,
)
from capsem_builder.gate.filesystem import write_text
from helpers.gate import RecordingJournal, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]


@pytest.fixture
def journal() -> RecordingJournal:
    return RecordingJournal()


@pytest.fixture
def context(journal: RecordingJournal) -> Context:
    return Context(RecordingRunner(PROJECT_ROOT), gate_config.load(PROJECT_ROOT), journal=journal)


# ---------------------------------------------------------------------------
# Atomic replacement
# ---------------------------------------------------------------------------


def test_runtime_text_writes_replace_instead_of_mutating_shared_inodes(tmp_path: Path) -> None:
    """Late-bound gate evidence must not rewrite another retained artifact.

    Run-ledger and digest contents only exist while the run is closing, so
    they use the function-form filesystem primitive rather than a plan
    action.  It still has to carry the same atomic-replacement property as the
    visible action: a hardlink must keep the old bytes and a symlink must be
    replaced, never followed into an unrelated run.
    """
    target = tmp_path / "DIGEST.md"
    target.write_text("old\n", encoding="utf-8")
    retained = tmp_path / "retained.md"
    os.link(target, retained)

    write_text(target, "new\n")

    assert target.read_text(encoding="utf-8") == "new\n"
    assert retained.read_text(encoding="utf-8") == "old\n"

    victim = tmp_path / "victim.md"
    victim.write_text("do not touch\n", encoding="utf-8")
    target.unlink()
    target.symlink_to(victim)

    write_text(target, "replacement\n")

    assert not target.is_symlink()
    assert target.read_text(encoding="utf-8") == "replacement\n"
    assert victim.read_text(encoding="utf-8") == "do not touch\n"


def test_atomic_replace_leaves_other_hardlinks_holding_the_old_bytes(
    context: Context, tmp_path: Path
) -> None:
    """The reason this primitive exists.

    The initrd is hash-named and hardlinked into every asset tree built from
    the same bytes. Rewriting it in place rewrites all of them, and the damage
    surfaces later as a VM that will not boot from a tree nobody touched.
    """
    target = tmp_path / "initrd.img"
    target.write_bytes(b"original")
    shared = tmp_path / "assets-by-hash" / "initrd.img"
    shared.parent.mkdir()
    os.link(target, shared)

    AtomicReplace(target, lambda scratch: scratch.write_bytes(b"rebuilt")).perform(context)

    assert target.read_bytes() == b"rebuilt"
    assert shared.read_bytes() == b"original", "the shared inode must be untouched"


def test_a_failed_build_leaves_the_target_and_no_scratch(context: Context, tmp_path: Path) -> None:
    """A half-written asset that looks whole is worse than a missing one."""
    target = tmp_path / "initrd.img"
    target.write_bytes(b"original")

    def explode(scratch: Path) -> None:
        scratch.write_bytes(b"partial")
        raise GateError("cpio failed")

    with pytest.raises(GateError, match="cpio failed"):
        AtomicReplace(target, explode).perform(context)

    assert target.read_bytes() == b"original"
    assert list(tmp_path.iterdir()) == [target], "the scratch file must be gone"


def test_atomic_replace_renders_the_target_without_touching_it(
    context: Context, tmp_path: Path
) -> None:
    target = tmp_path / "initrd.img"

    rendering = AtomicReplace(target, lambda scratch: None).render()

    assert "initrd.img" in rendering
    assert not target.exists()


# ---------------------------------------------------------------------------
# Hashing
# ---------------------------------------------------------------------------


def test_hash_records_the_artifact_in_the_journal(
    context: Context, journal: RecordingJournal, tmp_path: Path
) -> None:
    """So a run log answers "which bytes did this gate build" without
    re-hashing a tree that may already have been reclaimed."""
    artifact = tmp_path / "vmlinuz"
    artifact.write_bytes(b"kernel")

    Hash(artifact).perform(context)

    (path, digest, size) = journal.artifacts[0]
    assert path == artifact
    assert digest == blake3.blake3(b"kernel").hexdigest()
    assert size == len(b"kernel")


def test_hashing_a_missing_artifact_says_which_one(context: Context, tmp_path: Path) -> None:
    with pytest.raises(GateError, match=r"rootfs\.erofs"):
        Hash(tmp_path / "rootfs.erofs").perform(context)


def test_the_digest_comes_from_config_rather_than_the_call_site(
    context: Context,
) -> None:
    """One algorithm, named once. Two call sites disagreeing produces a log
    whose digests cannot be compared with each other."""
    assert context.config.runlog.artifact_digest == "blake3"


def test_an_unknown_digest_names_the_alternatives(
    journal: RecordingJournal, tmp_path: Path
) -> None:
    artifact = tmp_path / "vmlinuz"
    artifact.write_bytes(b"kernel")

    with pytest.raises(GateError) as failure:
        digest_of(artifact, algorithm="md5")

    assert "md5" in str(failure.value)
    assert "blake3" in str(failure.value)


# ---------------------------------------------------------------------------
# Symlinks
# ---------------------------------------------------------------------------


def test_symlink_replaces_an_existing_link(context: Context, tmp_path: Path) -> None:
    """`cache/target/assets/current` is repointed by whichever builder finished last, so
    the proof that follows has to aim it deliberately."""
    (tmp_path / "arm64").mkdir()
    (tmp_path / "x86_64").mkdir()
    link = tmp_path / "current"
    link.symlink_to("x86_64")

    Symlink(link, "arm64").perform(context)

    assert link.readlink().name == "arm64"


def test_symlink_verifies_where_it_ended_up(context: Context, tmp_path: Path) -> None:
    """Creating it and assuming is how the proof ran against the wrong
    architecture's assets and still passed."""
    link = tmp_path / "current"

    Symlink(link, "arm64").perform(context)

    assert link.readlink().name == "arm64"


def test_symlink_checks_where_it_landed_rather_than_assuming(
    context: Context, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Creating a link and trusting it is how a proof ran against the wrong
    architecture's assets and passed. The check is cheap; the miss is not."""
    monkeypatch.setattr(Path, "readlink", lambda _self: Path("x86_64"))

    with pytest.raises(GateError, match="points at"):
        Symlink(tmp_path / "current", "arm64").perform(context)


def test_symlink_refuses_to_replace_a_real_directory(context: Context, tmp_path: Path) -> None:
    """Removing a populated tree because a link was expected there is not a
    recoverable mistake."""
    occupied = tmp_path / "current"
    occupied.mkdir()
    (occupied / "vmlinuz").write_bytes(b"kernel")

    with pytest.raises(GateError, match="not a symlink"):
        Symlink(occupied, "arm64").perform(context)

    assert (occupied / "vmlinuz").exists()


# ---------------------------------------------------------------------------
# The plain ones
# ---------------------------------------------------------------------------


def test_make_dir_creates_parents_and_tolerates_existing(context: Context, tmp_path: Path) -> None:
    target = tmp_path / "a" / "b" / "c"

    MakeDir(target).perform(context)
    MakeDir(target).perform(context)

    assert target.is_dir()


def test_remove_takes_a_tree_or_a_file_and_tolerates_absence(
    context: Context, tmp_path: Path
) -> None:
    """Teardown runs against whatever state the failure left, which may be
    nothing at all."""
    tree = tmp_path / "run"
    (tree / "vm").mkdir(parents=True)
    (tree / "vm" / "serial.log").write_text("boot")
    single = tmp_path / "loose.log"
    single.write_text("x")

    Remove(tree).perform(context)
    Remove(single).perform(context)
    Remove(tmp_path / "never-existed").perform(context)

    assert not tree.exists()
    assert not single.exists()


def test_copy_handles_a_file_and_a_tree(context: Context, tmp_path: Path) -> None:
    source_tree = tmp_path / "built"
    (source_tree / "arm64").mkdir(parents=True)
    (source_tree / "arm64" / "vmlinuz").write_bytes(b"kernel")
    source_file = tmp_path / "manifest.json"
    source_file.write_text("{}")

    Copy(source_tree, tmp_path / "merged").perform(context)
    Copy(source_file, tmp_path / "merged" / "manifest.json").perform(context)

    assert (tmp_path / "merged" / "arm64" / "vmlinuz").read_bytes() == b"kernel"
    assert (tmp_path / "merged" / "manifest.json").read_text() == "{}"


def test_copying_a_tree_replaces_a_destination_root_symlink(
    context: Context, tmp_path: Path
) -> None:
    """The shared merge boundary must not write through its root argument."""
    source = tmp_path / "source"
    source.mkdir()
    (source / "new.log").write_text("new\n")
    unrelated = tmp_path / "unrelated"
    unrelated.mkdir()
    (unrelated / "old.log").write_text("old\n")
    target = tmp_path / "cache" / "target"
    target.parent.mkdir()
    target.symlink_to(unrelated, target_is_directory=True)

    Copy(source, target).perform(context)

    assert not target.is_symlink()
    assert (target / "new.log").read_text() == "new\n"
    assert (unrelated / "old.log").read_text() == "old\n"
    assert not (unrelated / "new.log").exists()


def test_copying_a_tree_refuses_a_source_root_symlink(context: Context, tmp_path: Path) -> None:
    """A caller cannot bypass the no-follow rule at the helper boundary."""
    real_source = tmp_path / "real-source"
    real_source.mkdir()
    (real_source / "evidence.log").write_text("evidence\n")
    source = tmp_path / "source"
    source.symlink_to(real_source, target_is_directory=True)

    with pytest.raises(GateError, match="source tree is a symlink"):
        Copy(source, tmp_path / "cache" / "target").perform(context)

    assert not (tmp_path / "cache" / "target").exists()


def test_copying_something_absent_says_which(context: Context, tmp_path: Path) -> None:
    with pytest.raises(GateError, match="vmlinuz"):
        Copy(tmp_path / "vmlinuz", tmp_path / "elsewhere").perform(context)


def test_require_file_names_what_is_missing(context: Context, tmp_path: Path) -> None:
    with pytest.raises(GateError, match=r"rootfs\.erofs"):
        RequireFile(tmp_path / "rootfs.erofs").perform(context)


def test_require_non_empty_catches_the_zero_length_artifact(
    context: Context, tmp_path: Path
) -> None:
    """A build that fails after creating its output leaves a file that passes
    every existence check -- which is how an empty rootfs reached a boot."""
    empty = tmp_path / "rootfs.erofs"
    empty.touch()

    with pytest.raises(GateError, match="empty"):
        RequireNonEmpty(empty).perform(context)


def test_require_non_empty_also_catches_the_absent_artifact(
    context: Context, tmp_path: Path
) -> None:
    """Missing and empty are different failures and deserve different words."""
    with pytest.raises(GateError, match="missing"):
        RequireNonEmpty(tmp_path / "rootfs.erofs").perform(context)


def test_require_non_empty_accepts_a_real_artifact(context: Context, tmp_path: Path) -> None:
    artifact = tmp_path / "rootfs.erofs"
    artifact.write_bytes(b"filesystem")

    RequireNonEmpty(artifact).perform(context)


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------


def test_no_file_action_touches_anything_when_rendered(tmp_path: Path) -> None:
    """The dry run must be free, including for the destructive ones."""
    victim = tmp_path / "run"
    (victim / "vm").mkdir(parents=True)

    renderings = [
        Remove(victim).render(),
        MakeDir(tmp_path / "new").render(),
        Copy(victim, tmp_path / "copied").render(),
        Symlink(tmp_path / "current", "arm64").render(),
        Hash(victim / "vmlinuz").render(),
        RequireFile(victim / "vmlinuz").render(),
        RequireNonEmpty(victim / "vmlinuz").render(),
    ]

    assert all(rendering for rendering in renderings)
    assert victim.exists()
    assert not (tmp_path / "new").exists()
    assert not (tmp_path / "copied").exists()
    assert not (tmp_path / "current").exists()


# ---------------------------------------------------------------------------
# Cleanup that reports what actually happened
# ---------------------------------------------------------------------------


def test_a_tree_that_cannot_be_removed_is_a_failure(tmp_path: Path, context) -> None:
    """`ignore_errors=True` made every removal succeed on paper.

    Stale benchmark, asset or run data then survives into the next
    qualification while the plan records the cleanup as done -- and retention
    reports bytes it reclaimed that are still on the disk. Absence is the
    tolerable outcome; a refusal is not.
    """
    tree = tmp_path / "held"
    (tree / "inner").mkdir(parents=True)
    (tree / "inner" / "file").write_text("x")
    tree.chmod(0o500)  # no write bit: the entry cannot be unlinked from
    try:
        with pytest.raises(GateError, match=str(tree.name)):
            Remove(tree).perform(context)
    finally:
        tree.chmod(0o700)


def test_a_path_that_was_never_there_is_not_a_failure(tmp_path: Path, context) -> None:
    """Teardown runs against whatever state a failure left behind, which may
    be nothing at all -- so absence is the expected case."""
    Remove(tmp_path / "never-existed").perform(context)


def test_a_file_that_cannot_be_removed_is_a_failure(tmp_path: Path, context) -> None:
    parent = tmp_path / "locked"
    parent.mkdir()
    victim = parent / "file"
    victim.write_text("x")
    parent.chmod(0o500)
    try:
        with pytest.raises(GateError, match="file"):
            Remove(victim).perform(context)
    finally:
        parent.chmod(0o700)

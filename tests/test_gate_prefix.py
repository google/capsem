"""The private copy of the checkout a run works from.

A gate that reads the tree everyone else is editing cannot prove anything about
a particular version of the software. This has cost four release runs: the last
one died at `source.verify` after 61 minutes because a second agent edited six
files in the checkout, and the observer had named the first intruder write 23
minutes before the run noticed.

The prefix closes it by construction rather than by detection -- the run reads
a tree nobody else has a path to. These tests hold the three properties that
make the copy usable as a subject: it is short enough for the sockets built
under it, it carries everything the run needs including the parts `git
ls-files` cannot see, and it reports the same revision as the tree it came
from.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest
from test_gate_socket_length import GATEWAY_SUFFIX, SUN_LEN

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _config():
    from capsem.gate import config as gate_config

    return gate_config.load(PROJECT_ROOT)


def _git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


@pytest.fixture
def source(tmp_path: Path) -> Path:
    """A checkout shaped like the real one, small enough to copy in a test.

    A real repository rather than a directory of files: the prefix carries
    `.git`, and the property that matters most -- that the copy reports the
    same revision -- is meaningless without one.
    """
    root = tmp_path / "src"
    (root / "crates" / "capsem-core").mkdir(parents=True)
    (root / "private" / "tauri").mkdir(parents=True)
    (root / "target" / "debug").mkdir(parents=True)

    (root / ".gitignore").write_text("target/\nprivate/\n", encoding="utf-8")
    (root / "tracked.txt").write_text("committed\n", encoding="utf-8")
    (root / "crates" / "capsem-core" / "lib.rs").write_text("fn main() {}\n", encoding="utf-8")
    # Gitignored, invisible to the digest, and load-bearing: this is the shape
    # of `private/tauri/`, whose keys the package lane signs with.
    (root / "private" / "tauri" / "key.pem").write_text("SECRET\n", encoding="utf-8")
    # Build output, which the prefix must not carry -- 164 GB and 84s in the
    # real checkout, against 186 MB and 2.2s without it.
    (root / "target" / "debug" / "huge.bin").write_text("x" * 4096, encoding="utf-8")

    # A tracked symlink pointing at a directory. `git ls-files` lists it like
    # any other entry, and this repository really has them -- `.agents/skills`
    # points at `skills/`, because agent-specific discovery mirrors the one
    # checked-in skill library rather than duplicating it.
    (root / ".agents").mkdir()
    (root / ".agents" / "skills").symlink_to(root / "crates", target_is_directory=True)

    _git(root, "init", "-q")
    _git(root, "config", "user.email", "gate@example.com")
    _git(root, "config", "user.name", "Gate")
    _git(root, "add", ".gitignore", "tracked.txt", "crates/capsem-core/lib.rs", ".agents/skills")
    _git(root, "commit", "-qm", "initial")
    # Uncommitted work, which the gate explicitly supports and the digest
    # counts: `git ls-files -co --exclude-standard`.
    (root / "untracked.txt").write_text("in progress\n", encoding="utf-8")
    (root / "tracked.txt").write_text("edited\n", encoding="utf-8")
    return root


# -- the budget --------------------------------------------------------------


def test_moving_the_run_into_a_prefix_cannot_lengthen_a_socket_path() -> None:
    """The gateway's sockets are built somewhere the prefix does not reach.

    `/tmp/capsem-gate-<32hex>` -- an early draft of this design -- is 41
    characters and reproduced the 12,024-error socket failure, because the
    gateway appends `instances/<uuid>-ws.sock` and macOS stops at 104 bytes.
    The escape is that the socket root is absolute and outside the checkout, so
    relocating the run adds nothing to it.

    Worth stating as its own property, because the obvious reading is wrong:
    the *workspace* run dir is relative to the checkout root, and at
    `<root>/target/test-home/.capsem/run` it is already 105 bytes with the
    gateway suffix -- over the limit today, prefix or no prefix. It is not the
    binding path, and a test that measured it would fail for a reason that has
    nothing to do with isolation. Mutation: point `[assets] run_dir_template`
    at a relative path and this goes red.
    """
    from capsem.gate import prefix

    config = _config()
    root = prefix.socket_root(config)

    assert root.is_absolute(), (
        f"{root} is relative, so it resolves inside the prefix and every "
        "terminal socket grows by the length of the prefix"
    )
    assert prefix.example(config) not in root.parents

    longest = len(str(root / "capsem-a.XXXXXX")) + 1 + GATEWAY_SUFFIX
    assert longest <= SUN_LEN, (
        f"a terminal socket would be {longest} bytes against a {SUN_LEN} limit"
    )


def test_the_prefix_is_no_more_expensive_than_the_checkout_it_replaces() -> None:
    """Relative, because the absolute budget is already spent elsewhere.

    Stated as a comparison rather than a constant so it keeps meaning if the
    checkout moves: whatever headroom exists today, a run from a prefix must
    not have less. Mutation: lengthen `[prefix] parent` and this goes red
    before anything has to boot a VM to find out.
    """
    from capsem.gate import prefix

    config = _config()
    allowance = len(str(config.root)) + 2
    assert len(str(prefix.example(config))) <= allowance, (
        f"the prefix is longer than {config.root} plus two characters, so it "
        "buys isolation by spending socket budget the asset lane needs"
    )


# -- what it carries ---------------------------------------------------------


def test_the_prefix_carries_the_working_tree_and_not_build_output(source: Path) -> None:
    """Every byte the source digest counts, and nothing that costs 164 GB.

    The digest is `git ls-files -co --exclude-standard`, so uncommitted edits
    and untracked non-ignored files are part of the subject. A prefix built
    from `HEAD` would qualify a different tree than the one being measured.
    """
    from capsem.gate import prefix

    target = source.parent / "prefix"
    prefix.populate(source, target, _config())

    assert (target / "tracked.txt").read_text(encoding="utf-8") == "edited\n"
    assert (target / "untracked.txt").is_file()
    assert (target / "crates" / "capsem-core" / "lib.rs").is_file()
    assert not (target / "target" / "debug" / "huge.bin").exists()


def test_a_tracked_symlink_stays_a_symlink(source: Path) -> None:
    """Copied as a link, not as whatever it points at.

    `git ls-files` lists symlinks the same as files, so a copy that resolves
    them either dies -- `cp` refuses a directory without `-R`, which is how
    this was found, against `.agents/skills` in the real checkout -- or
    silently duplicates a tree and produces a prefix whose digest can never
    match its source.
    """
    from capsem.gate import prefix

    target = source.parent / "prefix"
    prefix.populate(source, target, _config())

    link = target / ".agents" / "skills"
    assert link.is_symlink(), f"{link} was resolved instead of copied as a link"
    assert Path(os.readlink(link)) == Path(os.readlink(source / ".agents" / "skills"))


def test_the_prefix_carries_the_gitignored_paths_a_release_signs_with(source: Path) -> None:
    """`private/` is gitignored, so the digest cannot see it -- and the Tauri
    signing keys live there.

    Built from the digest set alone, a prefix silently loses them, and the
    first thing that notices is the package lane during a release. Declared in
    `[prefix] carried` for exactly this reason, alongside `.git`.
    """
    from capsem.gate import prefix

    target = source.parent / "prefix"
    prefix.populate(source, target, _config())

    assert (target / "private" / "tauri" / "key.pem").read_text(encoding="utf-8") == "SECRET\n"


def test_the_prefix_reports_the_same_revision_as_its_source(source: Path) -> None:
    """Dropping `.git` is the failure this catches.

    Build provenance goes through `build.rs`, and `RecordHead`,
    `RecordSourceState` and `_ForeignUidProbe` all shell out to git. Without
    it the copy is not a checkout, and the gate qualifies a revision it cannot
    name.
    """
    from capsem.gate import prefix

    target = source.parent / "prefix"
    prefix.populate(source, target, _config())

    assert (target / ".git").is_dir()
    assert _git(target, "rev-parse", "HEAD") == _git(source, "rev-parse", "HEAD")


def test_the_copy_is_independent_of_the_tree_it_came_from(source: Path) -> None:
    """The whole point, as an assertion.

    Clonefile is copy-on-write, not a hardlink: a write on either side must not
    be visible on the other. A hardlink-based copy passes every other test in
    this file and still lets an outside edit reach into a running gate, which
    is the exact failure the prefix exists to make impossible.
    """
    from capsem.gate import prefix

    target = source.parent / "prefix"
    prefix.populate(source, target, _config())

    (source / "tracked.txt").write_text("edited by someone else\n", encoding="utf-8")
    assert (target / "tracked.txt").read_text(encoding="utf-8") == "edited\n"

    assert os.stat(source / "tracked.txt").st_ino != os.stat(target / "tracked.txt").st_ino


# -- giving it back ----------------------------------------------------------


def test_a_finished_run_leaves_no_prefix(tmp_path: Path, source: Path) -> None:
    """Reclaimed on release, including the failure path.

    Each run costs ~100 MB. Left behind, a fortnight of gates is a disk-full
    in the middle of the next release rather than at a point where it is cheap.
    """
    from capsem.gate import config as gate_config
    from capsem.gate import prefix

    config = gate_config.load(PROJECT_ROOT).model_copy(
        update={"prefix": _config().prefix.model_copy(update={"parent": str(tmp_path)})}
    )
    target = tmp_path / "abcd1234"
    prefix.populate(source, target, config)
    assert target.is_dir()

    prefix.reclaim(config, target)
    assert not target.exists()


def test_reclaim_refuses_anything_that_is_not_a_prefix(tmp_path: Path) -> None:
    """A recursive delete of a path assembled in Python, so it is fenced.

    This is the shape the reclaimer guards exist to refuse, and the fence has
    to be containment rather than a shrug: an earlier draft checked that the
    path had a parent, which is true of every path on the system and would
    have deleted a checkout as happily as a prefix.
    """
    from capsem.gate import config as gate_config
    from capsem.gate import prefix
    from capsem.gate.errors import GateError

    config = gate_config.load(PROJECT_ROOT).model_copy(
        update={"prefix": _config().prefix.model_copy(update={"parent": str(tmp_path)})}
    )
    outsider = tmp_path.parent / "not-a-prefix"
    outsider.mkdir()

    with pytest.raises(GateError, match="refusing to reclaim"):
        prefix.reclaim(config, outsider)
    assert outsider.is_dir()

    # And the root itself, which is where every prefix lives.
    with pytest.raises(GateError, match="refusing to reclaim"):
        prefix.reclaim(config, tmp_path)
    assert tmp_path.is_dir()


def test_the_export_list_covers_what_a_release_publishes() -> None:
    """Everything built inside the prefix dies with it unless it is named here.

    `packages/` is the one that matters most and is easiest to forget: the
    signed `.pkg` a release publishes is built inside the run, so omitting it
    means a release that passes every gate and has nothing to ship.
    """
    exports = set(_config().prefix.exports)

    assert {"dist", "packages", "assets"} <= exports
    assert any(export.startswith("target/gate-runs") for export in exports), (
        "the run log is the evidence a failure is argued from, and it is written inside the prefix"
    )

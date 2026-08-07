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
from helpers.gate import RecordingRunner
from test_gate_socket_length import GATEWAY_SUFFIX, SUN_LEN

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _config():
    from capsem.gate import config as gate_config

    return gate_config.load(PROJECT_ROOT)


def _context(config):
    """A real `Context`, so the guard is exercised through its own signature.

    A `SimpleNamespace` stood in here and needed a type suppression to be
    passed at all -- which is the shape of a test that has drifted from what it
    claims to exercise. These two cases raise before anything reaches the
    runner; a recorder is there so that stops being true silently.
    """
    from capsem.gate.context import Context

    return Context(RecordingRunner(PROJECT_ROOT), config)


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

    # `.venv/` too, as the real checkout does: it is where an earlier run's
    # interpreter lives, and whether it is ignored decides whether a refresh
    # may delete it.
    (root / ".gitignore").write_text("target/\nprivate/\n.venv/\n", encoding="utf-8")
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
    # Never the developer's signing setup. This repository's global config
    # signs through 1Password, and when that agent is locked `git commit`
    # fails with `agent returned an error` and `failed to write commit
    # object` -- a fixture going red for a reason that has nothing to do with
    # what it tests, on the machine of whoever happens to have it configured.
    _git(root, "config", "commit.gpgsign", "false")
    _git(root, "config", "tag.gpgsign", "false")
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
    from capsem.gate import snapshot

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())

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
    from capsem.gate import snapshot

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())

    link = target / ".agents" / "skills"
    assert link.is_symlink(), f"{link} was resolved instead of copied as a link"
    assert Path(os.readlink(link)) == Path(os.readlink(source / ".agents" / "skills"))


def test_refreshing_an_existing_prefix_matches_the_source(source: Path) -> None:
    """The production resume path, not a second `populate`.

    An earlier version of this test called `populate()` twice while resuming
    actually called a different branch, so it proved a path nothing ran. It
    calls `refresh` now, which is what `--prefix` uses.

    Three things must hold. An overwrite must work at all -- `cp` handles it
    for regular files and `os.symlink` refuses an existing link, which killed
    the first real resume in under a second on `.agents/skills`. An edit must
    land. And a file *deleted* from the source must not survive: it did, so a
    resumed run compiled a tree the operator no longer had while its run log
    described the tree they thought they retried.
    """
    from capsem.gate import snapshot

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())
    assert (target / "untracked.txt").is_file()

    (source / "tracked.txt").write_text("fixed\n", encoding="utf-8")
    (source / "untracked.txt").unlink()
    snapshot.refresh(source, target, _config())

    assert (target / "tracked.txt").read_text(encoding="utf-8") == "fixed\n"
    assert (target / ".agents" / "skills").is_symlink()
    assert not (target / "untracked.txt").exists(), (
        "a file deleted from the source survived into the resumed tree"
    )


def test_the_prefix_carries_the_gitignored_paths_a_release_signs_with(source: Path) -> None:
    """`private/` is gitignored, so the digest cannot see it -- and the Tauri
    signing keys live there.

    Built from the digest set alone, a prefix silently loses them, and the
    first thing that notices is the package lane during a release. Declared in
    `[prefix] carried` for exactly this reason, alongside `.git`.
    """
    from capsem.gate import snapshot

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())

    assert (target / "private" / "tauri" / "key.pem").read_text(encoding="utf-8") == "SECRET\n"


def test_the_prefix_reports_the_same_revision_as_its_source(source: Path) -> None:
    """Dropping `.git` is the failure this catches.

    Build provenance goes through `build.rs`, and `RecordHead`,
    `RecordSourceState` and `_ForeignUidProbe` all shell out to git. Without
    it the copy is not a checkout, and the gate qualifies a revision it cannot
    name.
    """
    from capsem.gate import snapshot

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())

    assert (target / ".git").is_dir()
    assert _git(target, "rev-parse", "HEAD") == _git(source, "rev-parse", "HEAD")


def test_the_copy_is_the_source_at_one_instant(source: Path) -> None:
    """A faithful copy digests identically to the tree it came from.

    Not a restatement of the tests above. Those check named paths; this checks
    the *set*, its contents and its modes all at once, using the same measure
    `source.record` writes down and `source.verify` re-asserts an hour later.
    An edit landing during the copy would otherwise produce a mixed tree --
    some files from before it and some from after -- which becomes the stable
    subject of the whole run and passes `source.verify` happily, even though
    that combination of bytes never existed at any instant in the checkout.
    """
    from capsem.gate import snapshot

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())

    config = _config()
    assert snapshot.digest(target, config) == snapshot.digest(source, config)


def test_a_copy_taken_while_the_source_moved_is_refused(source: Path, monkeypatch) -> None:
    """The race, injected at a real seam rather than described.

    The window is small -- 2.2s for this repository -- and it is not zero, so
    the copy has to be checked rather than assumed. Refused loudly: retrying
    costs seconds, and a torn subject costs the hour it takes to qualify it.
    """
    from capsem.gate import snapshot
    from capsem.gate.errors import GateError

    faithful = snapshot._copy_files

    def edit_the_source_midway(origin: Path, into: Path, relatives: list[Path]) -> None:
        faithful(origin, into, relatives)
        (origin / "tracked.txt").write_text("landed during the copy\n", encoding="utf-8")

    monkeypatch.setattr(snapshot, "_copy_files", edit_the_source_midway)

    with pytest.raises(GateError, match="while its private copy was being made"):
        snapshot.populate(source, source.parent / "prefix", _config())


def test_a_refresh_keeps_what_the_earlier_run_built(source: Path) -> None:
    """The whole reason to reuse a tree, and the first version deleted it.

    `refresh` removes what the source no longer names, so a file deleted from
    the checkout cannot survive into a resumed run. The first implementation
    walked the whole tree and spared a hand-written list of exports and carried
    paths -- which meant everything *else* gitignored was fair game, including
    `.venv`. A real resume died before its first step: "Project virtual
    environment ... no Python executable was found".

    Asked with the command that defines the subject instead, an ignored path is
    never a candidate. That is one definition of "what this tree is", used by
    the digest, by the copy and now by the deletion pass.
    """
    from capsem.gate import snapshot

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())

    # What an earlier run would have built: all ignored, none in the subject.
    (target / ".venv" / "bin").mkdir(parents=True)
    (target / ".venv" / "bin" / "python").write_text("#!/bin/sh\n", encoding="utf-8")
    (target / "target" / "debug").mkdir(parents=True, exist_ok=True)
    (target / "target" / "debug" / "built.bin").write_text("artifact\n", encoding="utf-8")

    (source / "untracked.txt").unlink()
    snapshot.refresh(source, target, _config())

    assert (target / ".venv" / "bin" / "python").is_file(), (
        "the venv an earlier run built was deleted, so the resumed run has no "
        "interpreter -- which is the entire cost the prefix exists to avoid"
    )
    assert (target / "target" / "debug" / "built.bin").is_file()
    assert not (target / "untracked.txt").exists(), "and the deletion pass still works"


def test_a_refresh_that_did_not_converge_is_refused(source: Path, monkeypatch) -> None:
    """Resume gets the same check, for a sharper reason.

    `refresh` has more ways to be wrong than `populate`: it overwrites, and it
    has to *remove* what the source no longer names. A file it failed to delete
    leaves a resumed run compiling a tree the operator no longer has, which is
    exactly the defect the deletion pass was added for -- and nothing but this
    would notice the pass regressing.
    """
    from capsem.gate import snapshot
    from capsem.gate.errors import GateError

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())

    (source / "untracked.txt").unlink()
    # A deletion pass that stops removing anything: the copy then keeps a file
    # the source no longer has, and the digests diverge.
    monkeypatch.setattr(snapshot, "_subject", lambda tree: [])

    with pytest.raises(GateError, match="while its private copy was being made"):
        snapshot.refresh(source, target, _config())


def test_the_copy_is_independent_of_the_tree_it_came_from(source: Path) -> None:
    """The whole point, as an assertion.

    Clonefile is copy-on-write, not a hardlink: a write on either side must not
    be visible on the other. A hardlink-based copy passes every other test in
    this file and still lets an outside edit reach into a running gate, which
    is the exact failure the prefix exists to make impossible.
    """
    from capsem.gate import snapshot

    target = source.parent / "prefix"
    snapshot.populate(source, target, _config())

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
    from capsem.gate import prefix, snapshot

    config = gate_config.load(PROJECT_ROOT).model_copy(
        update={"prefix": _config().prefix.model_copy(update={"parent": str(tmp_path)})}
    )
    target = tmp_path / "abcd1234"
    snapshot.populate(source, target, config)
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


def test_a_process_already_inside_a_prefix_builds_no_second_one(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The recursion guard, and the only thing that stops it.

    The parent runs `capsem-gate` again inside the copy, so without a marker
    the child copies the copy, forever. `source_checkout` returning non-`None`
    is what the command hook tests, and it doubles as the answer to "which tree
    was this copied from" that `require-source-unchanged` needs.
    """
    from capsem.gate import prefix

    config = _config()
    monkeypatch.delenv(config.environment.source_checkout, raising=False)
    assert prefix.source_checkout(config) is None

    monkeypatch.setenv(config.environment.source_checkout, "/Users/someone/git/capsem")
    assert prefix.source_checkout(config) == Path("/Users/someone/git/capsem")


def test_the_release_guard_still_sees_the_real_branch_move(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """`require-source-unchanged` must not go vacuous against a private copy.

    This is the guard the plan warned would be lost with no test going red. A
    prefix is frozen when it is made, so its own `HEAD` and digest can never
    change from outside -- comparing only those would pass unconditionally
    while a commit landed on the branch being qualified, and the gate would
    publish a qualification for a revision it never tested.

    So the recorded state carries the *source* checkout's `HEAD` too, and this
    asserts the comparison actually fires on it. Mutation: drop the
    `source_head` branch from `RequireSourceUnchanged` and this goes green
    again, which is exactly the silence being prevented.
    """
    import json

    from capsem.gate import config as gate_config
    from capsem.gate.errors import GateError
    from capsem.gate.sourcestate import RequireSourceUnchanged

    config = gate_config.load(PROJECT_ROOT)
    record = tmp_path / "source-state.json"
    record.write_text(
        json.dumps(
            {
                "head": "frozen",
                "source_head": "before",
                "digest": "same",
                "gate_source": "x",
            }
        ),
        encoding="utf-8",
    )

    # Everything a prefix can see is unchanged; only the tree it was copied
    # from moved.
    measured = {"head": "frozen", "source_head": "after", "digest": "same", "gate_source": "x"}
    monkeypatch.setattr("capsem.gate.sourcestate._measure", lambda context: measured)
    monkeypatch.setattr("capsem.gate.sourcestate._record_file", lambda context: record)

    with pytest.raises(GateError, match="copied from moved"):
        RequireSourceUnchanged().perform(_context(config))


def test_the_release_guard_still_sees_the_real_tree_edited(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """`HEAD` is not the whole subject, and dirty-tree qualification is why.

    The gate deliberately supports uncommitted work, so a checkout can be
    edited without `HEAD` moving at all -- which is the ordinary case, not an
    exotic one. Against a prefix, the run's own digest is frozen and the
    source's `HEAD` is unchanged, so both other comparisons pass while the tree
    being released is edited underneath. Only the originating checkout's digest
    can see it.
    """
    import json

    from capsem.gate import config as gate_config
    from capsem.gate.errors import GateError
    from capsem.gate.sourcestate import RequireSourceUnchanged

    config = gate_config.load(PROJECT_ROOT)
    record = tmp_path / "source-state.json"
    frozen = {"head": "frozen", "source_head": "still", "digest": "same", "gate_source": "x"}
    record.write_text(json.dumps({**frozen, "source_digest": "before"}), encoding="utf-8")

    measured = {**frozen, "source_digest": "after"}
    monkeypatch.setattr("capsem.gate.sourcestate._measure", lambda context: measured)
    monkeypatch.setattr("capsem.gate.sourcestate._record_file", lambda context: record)

    with pytest.raises(GateError, match="copied from was edited"):
        RequireSourceUnchanged().perform(_context(config))


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


def test_the_built_binaries_are_every_host_binary() -> None:
    """A new crate with a binary joins the build list, or this fails.

    Three consecutive runs from a clean checkout each died on one missing
    binary -- `capsem` at `codesign`, then `capsem-mcp-aggregator` at the VM
    boot, then `capsem-tray` in the build-chain suite. Each fix added the one
    name the last failure happened to reach, and each cost a twenty-minute run
    to find the next.

    So the list is checked against `cargo metadata` instead of against
    yesterday's failure. The two exclusions are declared rather than implied:
    `capsem-app` embeds `frontend/dist` and belongs to `build-ui`, which builds
    the bundle first, and the guest crate's binaries are musl and belong to
    `initrd.guest-agents`.
    """
    import json
    import subprocess

    config = _config()
    settings = config.signing

    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=PROJECT_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    host = {
        target["name"]
        for package in metadata["packages"]
        for target in package["targets"]
        if "bin" in target["kind"] and package["name"] != settings.guest_crate
    }

    expected = host - set(settings.built_elsewhere)
    missing = sorted(expected - set(settings.built))
    assert not missing, (
        "these host binaries are built by nothing, so a run that does not "
        f"inherit a warm checkout will fail on the first one it needs: {missing}"
    )

    stale = sorted(set(settings.built) - host)
    assert not stale, f"these are in the build list but no longer exist: {stale}"


def test_a_sweep_keeps_the_newest_and_reclaims_the_rest(tmp_path: Path) -> None:
    """Bounded growth, on the way in.

    A failed run keeps its tree so it can be resumed into, which means nothing
    on the failure path deletes it -- and `[disk] reclaimable` only accepts
    paths inside the checkout, so `gc` never reached these. One retained prefix
    on this machine was 22 GiB and carried the copied signing material with it.

    Swept on entry rather than on exit, the same shape as `[workspace] home`:
    the run *after* a failure is the one that no longer needs its tree.
    """
    import time

    from capsem.gate import config as gate_config
    from capsem.gate import prefix

    config = gate_config.load(PROJECT_ROOT).model_copy(
        update={"prefix": _config().prefix.model_copy(update={"parent": str(tmp_path), "keep": 1})}
    )
    older, newer = tmp_path / "aaaaaaaa", tmp_path / "bbbbbbbb"
    for path in (older, newer):
        path.mkdir()
        (path / "bulk").write_text("x", encoding="utf-8")
        time.sleep(0.01)

    reclaimed = prefix.sweep(config)

    assert reclaimed == [older]
    assert not older.exists()
    assert newer.is_dir(), "the newest survives, so a failed run can still be resumed"


def test_reclaim_does_not_report_success_on_a_tree_it_left_behind(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`ignore_errors=True` is right for a chmodded tree and wrong as the last
    word: a successful run that silently kept its copy is how the disk fills
    with nothing reporting it."""
    from capsem.gate import config as gate_config
    from capsem.gate import prefix
    from capsem.gate.errors import GateError

    config = gate_config.load(PROJECT_ROOT).model_copy(
        update={"prefix": _config().prefix.model_copy(update={"parent": str(tmp_path), "keep": 1})}
    )
    stubborn = tmp_path / "cccccccc"
    stubborn.mkdir()

    monkeypatch.setattr(prefix.shutil, "rmtree", lambda *a, **k: None)
    with pytest.raises(GateError, match="could not reclaim"):
        prefix.reclaim(config, stubborn)


def test_a_linked_worktree_is_refused_rather_than_half_isolated(tmp_path: Path) -> None:
    """`.git` is a file in a worktree, and copying it copies a pointer.

    The copy then follows the *original* repository: a commit over there moves
    the supposedly private prefix's `HEAD`, and the isolation silently becomes
    the detection it was built to replace. This repository really uses linked
    worktrees, under `.claude/worktrees/`, so the case is reachable.

    Refused rather than repaired -- making the copy self-contained means
    reproducing the common object store, and a loud message naming the main
    checkout beats a private tree that quietly is not one.
    """
    from capsem.gate import snapshot
    from capsem.gate.errors import GateError

    worktree = tmp_path / "linked"
    worktree.mkdir()
    (worktree / ".git").write_text(f"gitdir: {tmp_path}/main/.git/worktrees/linked\n")

    with pytest.raises(GateError, match="linked worktree"):
        snapshot.populate(worktree, tmp_path / "prefix", _config())

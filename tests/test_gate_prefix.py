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


def test_the_prefix_example_reserves_the_full_release_commit() -> None:
    """The longest identity is one full commit, not a random abbreviation."""
    from capsem.gate import prefix

    config = _config()
    assert prefix.example(config).name == "0" * 40

    from capsem.gate.workspace import Workspace

    socket = Workspace(config).run_dir / config.service.socket
    assert len(os.fsencode(socket)) + 1 <= SUN_LEN


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
    `RecordSourceState` and the provenance step all shell out to git. Without
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


def test_export_materializes_the_selected_assets_without_copying_current(
    tmp_path: Path,
) -> None:
    """The private gate selects a verified profile with a top-level symlink.

    Export must dereference that one selector into the checkout while retaining
    the self-contained ``assets/current`` architecture selector. Dereferencing
    both materializes a second multi-gigabyte asset tree.
    """
    from capsem.gate import config as gate_config

    checkout = tmp_path / "checkout"
    private = tmp_path / "private"
    for root in (checkout, private):
        (root / "config").mkdir(parents=True)
        (root / "config" / "gate.toml").write_text(
            (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"),
            encoding="utf-8",
        )
    selected = private / "target" / "ironbank-assets" / "code" / "assets"
    architecture = selected / "x86_64"
    architecture.mkdir(parents=True)
    (architecture / "rootfs.erofs").write_bytes(b"fresh")
    (selected / "manifest.json").write_text('{"fresh":true}\n')
    (selected / "current").symlink_to("x86_64")
    (private / "assets").symlink_to("target/ironbank-assets/code/assets")
    private_config = private / "target" / "config"
    private_config_manifest = private_config / "assets" / "manifest.json"
    private_config_manifest.parent.mkdir(parents=True)
    private_config_manifest.write_text('{"fresh":true}\n')

    old = checkout / "assets"
    (old / "current").mkdir(parents=True)
    (old / "current" / "stale").write_text("stale\n")
    (old / "stale").write_text("stale\n")
    old_config = checkout / "target" / "config"
    old_config_manifest = old_config / "assets" / "manifest.json"
    old_config_manifest.parent.mkdir(parents=True)
    old_config_manifest.write_text('{"stale":true}\n')
    (old_config / "retired").mkdir()

    from capsem.gate import buildcache

    buildcache.export(private, checkout, gate_config.load(private))

    assert not old.is_symlink()
    assert not (old / "stale").exists()
    assert (old / "manifest.json").read_text() == '{"fresh":true}\n'
    assert (old / "current").is_symlink()
    assert (old / "current").readlink() == Path("x86_64")
    assert old_config_manifest.read_text() == '{"fresh":true}\n'
    assert not (old_config / "retired").exists()


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

    assert {"dist", "packages", "assets", "target/config"} <= exports
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


def _own_checkout(tmp_path: Path) -> Path:
    """An empty checkout, so a driven run has nothing expensive to adopt.

    A run fills an empty cache from what the checkout exported, and this
    repository's build trees are gigabytes. Six unit tests pointed at the real
    checkout copied them six times and filled a 32 GiB tmpfs.
    """
    checkout = tmp_path / "checkout"
    checkout.mkdir(exist_ok=True)
    return checkout


def test_a_successful_reused_prefix_stays_available_for_the_next_continuation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A focused diagnostic is not the complete candidate it is helping fix.

    The first successful ``test-functional --prefix ... --from ...`` deleted
    the retained candidate tree.  Its functional evidence was valid, but the
    next ``candidate --prefix ...`` could no longer reuse the fifty-nine steps
    that had already run.  Naming an existing prefix is an iteration contract:
    success exports its evidence, while the tree remains until a later fresh
    run sweeps it.
    """
    from capsem.gate import config as gate_config
    from capsem.gate import prefix

    reused = tmp_path / "aaaaaaaa"
    reused.mkdir()
    config = gate_config.load(PROJECT_ROOT).model_copy(
        update={"prefix": _config().prefix.model_copy(update={"parent": str(tmp_path), "keep": 1})}
    )
    reclaimed: list[Path] = []

    class SuccessfulRunner:
        def note(self, message: str) -> None:
            if message.startswith("prefix kept"):
                assert message == f"prefix kept for resuming: {reused}"

        def run(self, *args, **kwargs) -> int:
            return 0

    from capsem.gate import buildcache

    config = config.model_copy(update={"root": _own_checkout(tmp_path)})
    monkeypatch.setattr(prefix.snapshot, "refresh", lambda *args: None)
    monkeypatch.setattr(buildcache, "export", lambda *args: None)
    monkeypatch.setattr(prefix, "reclaim", lambda _config, path: reclaimed.append(path))

    assert (
        prefix.run_from_private_copy(SuccessfulRunner(), config, ["test-functional"], reuse=reused)
        == 0
    )
    assert reclaimed == []
    assert reused.is_dir()


def test_a_fresh_successful_prefix_is_still_reclaimed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Successful fresh qualification has no continuation debt to retain."""
    from capsem.gate import config as gate_config
    from capsem.gate import prefix

    fresh = tmp_path / "bbbbbbbb"
    config = gate_config.load(PROJECT_ROOT).model_copy(
        update={"prefix": _config().prefix.model_copy(update={"parent": str(tmp_path), "keep": 1})}
    )
    reclaimed: list[Path] = []

    class SuccessfulRunner:
        def note(self, message: str) -> None:
            if message.startswith("prefix kept"):
                raise AssertionError(f"fresh success should not be retained: {message}")

        def run(self, *args, **kwargs) -> int:
            return 0

    from capsem.gate import buildcache

    config = config.model_copy(update={"root": _own_checkout(tmp_path)})
    monkeypatch.setattr(prefix, "allocate", lambda *args: fresh)
    monkeypatch.setattr(prefix, "sweep", lambda *args: [])
    monkeypatch.setattr(prefix.snapshot, "populate", lambda *args: fresh.mkdir())
    monkeypatch.setattr(buildcache, "export", lambda *args: None)
    monkeypatch.setattr(prefix, "reclaim", lambda _config, path: reclaimed.append(path))

    assert prefix.run_from_private_copy(SuccessfulRunner(), config, ["candidate"]) == 0
    assert reclaimed == [fresh]


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


def test_a_linked_worktree_gets_a_repository_of_its_own(tmp_path: Path) -> None:
    """A worktree is copyable, and the copy does not follow the original.

    This used to be refused. `.git` in a linked worktree is a *file* holding an
    absolute `gitdir:` path, so carrying it like any other path left the prefix
    attached to live metadata -- a commit in the original moved the supposedly
    private HEAD. The refusal was correct about the hazard and wrong about the
    remedy: it made the gate unrunnable from a worktree, and worktrees are how
    an agent gets an isolated tree, so the isolation machinery refused to run
    for precisely the people it was built for.

    Driven through a real `git worktree`, not a hand-written `.git` file. The
    previous version of this test wrote a pointer at a path that did not exist,
    which every git command rejected for the wrong reason.
    """
    from capsem.gate import snapshot

    origin = tmp_path / "origin"
    origin.mkdir()
    _git(origin, "init", "-q", "-b", "main")
    _git(origin, "config", "user.email", "t@example.com")
    _git(origin, "config", "user.name", "t")
    (origin / "tracked.txt").write_text("one\n")
    _git(origin, "add", "tracked.txt")
    _git(origin, "commit", "-qm", "first")

    linked = tmp_path / "linked"
    _git(origin, "worktree", "add", "-q", "--detach", str(linked))
    assert (linked / ".git").is_file(), "the fixture is not a linked worktree"
    before = _git(linked, "rev-parse", "HEAD")

    prefix = tmp_path / "prefix"
    snapshot.populate(linked, prefix, _config())

    assert (prefix / ".git").is_dir(), "the prefix did not get a repository of its own"
    assert _git(prefix, "rev-parse", "HEAD") == before
    assert _git(prefix, "ls-files") == "tracked.txt", "the index was not populated from HEAD"

    # Self-contained: hardlinked objects, no `alternates` pointing back. A
    # borrowed object store would let a `gc` in the original prune bytes out
    # from under a running gate.
    assert not (prefix / ".git" / "objects" / "info" / "alternates").exists()

    # The property the old refusal was protecting: the original moves on, and
    # the copy does not notice.
    (origin / "tracked.txt").write_text("two\n")
    _git(origin, "commit", "-qam", "second")
    assert _git(prefix, "rev-parse", "HEAD") == before, (
        "a commit in the original moved the prefix's HEAD; the copy is not private"
    )


def test_repository_copy_does_not_hardlink_across_filesystems(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Inspection checkouts may live on a different device from the source."""
    from capsem.gate import snapshot

    source = tmp_path / "source"
    source.mkdir()
    _git(source, "init", "-q", "-b", "main")
    _git(source, "config", "user.email", "t@example.com")
    _git(source, "config", "user.name", "t")
    (source / "tracked.txt").write_text("one\n")
    _git(source, "add", "tracked.txt")
    _git(source, "commit", "-qm", "first")

    target = tmp_path / "prefix"
    target.mkdir()
    real_stat = Path.stat

    def other_device(path: Path, *args, **kwargs):
        result = real_stat(path, *args, **kwargs)
        if path == target.parent:
            values = list(result)
            values[2] += 1
            return os.stat_result(values)
        return result

    monkeypatch.setattr(Path, "stat", other_device)
    snapshot._materialize_repository(source, target)

    assert _git(target, "rev-parse", "HEAD") == _git(source, "rev-parse", "HEAD")
    assert _git(target, "ls-files") == "tracked.txt"
    assert not (target / ".git" / "objects" / "info" / "alternates").exists()


def _git(cwd: Path, *args: str) -> str:
    import subprocess

    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def test_exporting_a_run_cannot_write_through_a_symlink(tmp_path: Path) -> None:
    """An export must never follow a symlink, on either side.

    This destroyed run logs. `target/gate-runs/latest` points at the newest
    run, in the private tree and on the host alike, and
    `shutil.copytree(..., dirs_exist_ok=True)` dereferences the source link and
    then writes the contents through the destination link -- into an unrelated
    older run, replacing every file in it with `copy2`, timestamps and all. The
    result is a well-formed log describing a run that never happened in it, and
    `source.verify` and the timing ratchet both read these logs as evidence.

    Driven through `export` rather than the helper it delegates to. Aimed at
    the helper this passed with the defective call restored, because the
    defect was never in the copying -- it was in which copy `export` chose.

    Asserted on the outcome rather than on arguments, too: `symlinks=True`
    alone stops the dereference and then raises because the destination link
    exists, so a contract about the flag would have accepted a fix that fails
    every export.
    """

    config = _config()
    runs = config.runlog.root

    private = tmp_path / "prefix"
    (private / runs / "run-new").mkdir(parents=True)
    (private / runs / "run-new" / "run.jsonl").write_text("the run that just finished\n")
    (private / runs / "latest").symlink_to("run-new")

    host = tmp_path / "host"
    (host / runs / "run-old").mkdir(parents=True)
    (host / runs / "run-old" / "run.jsonl").write_text("an older, unrelated run\n")
    (host / runs / "latest").symlink_to("run-old")

    from capsem.gate import buildcache

    buildcache.export(private, host, config)

    assert (host / runs / "run-old" / "run.jsonl").read_text() == "an older, unrelated run\n", (
        "the export wrote through the destination `latest` symlink and "
        "destroyed the unrelated run it pointed at"
    )
    assert (host / runs / "run-new" / "run.jsonl").read_text() == "the run that just finished\n"
    # Replaced as a link, not materialized into a directory of copied files.
    assert (host / runs / "latest").is_symlink()
    assert os.readlink(host / runs / "latest") == "run-new"


def _merge_case(root: Path, name: str) -> tuple[Path, Path]:
    source, target = root / "src", root / "dst"
    source.mkdir(parents=True)
    if name == "target-is-symlink":
        # The root is the level the first fix missed: clearing links only
        # during the descent left the entry point writing through one.
        (source / "f.txt").write_text("new")
        victim = root / "real"
        victim.mkdir()
        (victim / "f.txt").write_text("victim")
        target.symlink_to("real")
    elif name == "link-over-real-dir":
        (source / "e").symlink_to("elsewhere")
        target.mkdir()
        (target / "e").mkdir()
    elif name == "link-over-real-file":
        (source / "e").symlink_to("elsewhere")
        target.mkdir()
        (target / "e").write_text("displaced")
    elif name == "dir-over-link":
        (source / "d").mkdir()
        (source / "d" / "f").write_text("new")
        target.mkdir()
        victim = target / "real"
        victim.mkdir()
        (victim / "f").write_text("victim")
        (target / "d").symlink_to("real")
    return source, target


@pytest.mark.parametrize(
    "case",
    ["target-is-symlink", "link-over-real-dir", "link-over-real-file", "dir-over-link"],
)
def test_merging_a_tree_never_writes_through_a_link(case: str, tmp_path: Path) -> None:
    """Every place a symlink can sit, on either side, at any depth.

    Written as a matrix because the first fix passed the case it was written
    for and failed three others: the root was never cleared, so
    `merge_tree(origin, target)` with a symlinked `target` reproduced the exact
    defect; and `os.symlink` refuses *any* existing name, so a source link
    landing on a real file or directory raised instead of merging.
    """
    from capsem.gate.filesystem import merge_tree

    source, target = _merge_case(tmp_path, case)
    merge_tree(source, target)

    # Nothing reachable only through a link may have been rewritten.
    survivors = [p for p in tmp_path.rglob("*") if p.is_file() and p.read_text() == "victim"]
    if case in {"target-is-symlink", "dir-over-link"}:
        assert survivors, f"{case}: the merge wrote through a link and destroyed the target"
    if case in {"link-over-real-dir", "link-over-real-file"}:
        assert (target / "e").is_symlink(), f"{case}: the source link did not survive as a link"


def test_merging_keeps_what_the_target_already_had(tmp_path: Path) -> None:
    """Merging is not replacing -- the sibling that arrived first stays."""
    from capsem.gate.filesystem import merge_tree

    source, target = tmp_path / "src", tmp_path / "dst"
    source.mkdir()
    target.mkdir()
    (source / "arriving").write_text("second")
    (target / "already-here").write_text("first")

    merge_tree(source, target)
    assert sorted(p.name for p in target.iterdir()) == ["already-here", "arriving"]


def test_copying_a_tree_keeps_a_symlink_a_symlink(tmp_path: Path) -> None:
    """`copy_tree` must not dereference, for the same reason `merge_tree` must not.

    The write-through defect cannot occur here -- the target is removed first,
    so nothing is left to write through. Dereferencing the *source* is the
    separate hazard: `assets/current` is a relative selector into a
    multi-gigabyte architecture, and materializing it copies the whole tree for
    no new bytes.

    This used to be a `symlinks=` argument defaulting to False that the only
    informed caller overrode, which is a trap set for the next caller.
    """
    from capsem.gate.filesystem import copy_tree

    source = tmp_path / "src"
    (source / "arch").mkdir(parents=True)
    (source / "arch" / "big.bin").write_bytes(b"payload")
    (source / "current").symlink_to("arch")

    target = tmp_path / "dst"
    copy_tree(source, target)

    assert (target / "current").is_symlink(), "copy_tree dereferenced the selector"
    assert os.readlink(target / "current") == "arch"
    assert (target / "arch" / "big.bin").read_bytes() == b"payload"


def _capped(tmp_path: Path, cap_gb: float):
    """A config whose prefix root and shared build root are both disposable."""
    original = _config()
    return original.model_copy(
        update={
            "prefix": original.prefix.model_copy(
                update={
                    "parent": str(tmp_path / "prefixes"),
                    "cargo_target": "{parent}-target",
                    "cargo_target_max_gb": cap_gb,
                }
            )
        }
    )


def test_the_shared_build_directory_is_measured_and_kept_under_its_cap(
    tmp_path: Path,
) -> None:
    """Under cap it is reported and left alone; over cap it goes, whole.

    The one part of this system nothing reclaimed. Cargo never garbage-collects,
    so without a bound the directory only grows -- and `[disk] required_free_gb`
    is a floor on the filesystem rather than a bound on this, which means it
    reports that something already ate the disk instead of stopping the growth.
    """
    from capsem.gate import cargotarget

    config = _capped(tmp_path, cap_gb=1.0)
    shared = cargotarget.path(config)
    (shared / "debug").mkdir(parents=True)
    (shared / "debug" / "libcapsem.rlib").write_bytes(b"\0" * 4096)

    held = cargotarget.bound(config)
    assert not held.discarded, "a directory well under its cap must survive"
    assert 0 < held.gb < 1.0
    assert (shared / "debug" / "libcapsem.rlib").exists()

    # The same tree, against a cap it cannot fit under.
    over = cargotarget.bound(_capped(tmp_path, cap_gb=4096 / 1024**3 / 2))
    assert over.discarded
    assert not shared.exists(), (
        "the cap discards the build directory whole -- cargo decides staleness "
        "by fingerprint, so removing chosen files underneath it corrupts that"
    )


def test_measuring_the_build_directory_does_not_follow_the_prefixes_into_it(
    tmp_path: Path,
) -> None:
    """Every prefix points into this tree; counting through them double-bills.

    `target/debug` in each prefix is a symlink to the shared root. A size that
    followed links would bill the same bytes once per run on disk and discard a
    directory that had not grown at all.
    """
    from capsem.gate import cargotarget

    config = _capped(tmp_path, cap_gb=1.0)
    shared = cargotarget.path(config)
    (shared / "debug").mkdir(parents=True)
    (shared / "debug" / "libcapsem.rlib").write_bytes(b"\0" * 8192)
    alone = cargotarget.bound(config).gb

    prefix_path = tmp_path / "prefixes" / ("0" * 8)
    cargotarget.link_profiles(config, prefix_path)
    assert (prefix_path / "target" / "debug").is_symlink()
    # The link now resolves into the shared tree; the measurement must not.
    assert cargotarget.bound(config).gb == alone


def test_a_lease_outlives_its_prefix_only_until_the_next_sweep(tmp_path: Path) -> None:
    """127 of these had accumulated, one per identity ever run.

    Zero bytes each, so this is not about space: it is that the directory
    holding the prefixes stops being readable at a glance, and that listing is
    where a prefix nobody reclaimed gets noticed.
    """
    from capsem.gate.prefixlease import reclaim_orphan_leases

    config = _capped(tmp_path, cap_gb=1.0)
    root = Path(config.prefix.parent)
    root.mkdir(parents=True)
    live = root / ("a" * 8)
    live.mkdir()
    for identity in (live.name, "b" * 8):
        (root / config.prefix.lease_template.format(identity=identity)).touch()

    reclaimed = reclaim_orphan_leases(config)

    assert [path.name for path in reclaimed] == [
        config.prefix.lease_template.format(identity="b" * 8)
    ], "only the lease whose prefix is gone may be removed"
    assert (root / config.prefix.lease_template.format(identity=live.name)).exists()


def test_a_held_lease_is_never_unlinked_from_under_its_owner(tmp_path: Path) -> None:
    """The file *is* the mutual exclusion, so a busy one is skipped.

    Unlinking it would leave the holder locked on an unreachable inode while
    the next run creates a fresh file and locks that one too, and both would
    believe they owned the prefix.
    """
    from capsem.gate.prefixlease import lease, reclaim_orphan_leases

    config = _capped(tmp_path, cap_gb=1.0)
    root = Path(config.prefix.parent)
    root.mkdir(parents=True)
    gone = root / ("c" * 8)
    name = config.prefix.lease_template.format(identity=gone.name)
    (root / name).touch()

    with lease(config, gone):
        assert reclaim_orphan_leases(config) == []
        assert (root / name).exists()

    assert [path.name for path in reclaim_orphan_leases(config)] == [name]


def test_a_pulled_lane_finds_its_binaries_where_every_test_looks(tmp_path: Path) -> None:
    """`target/debug` is the one place the test tree resolves a host binary.

    Roughly twenty-five checked-in modules spell `PROJECT_ROOT/target/debug/
    <name>`, and they are not wrong to: a test should not have to know whether
    this run built its binaries or was handed them. In a prefix carrying only
    tracked files that directory is empty, which took down three binary-release
    dispatches, each found one file at a time with `--maxfail=5` hiding the
    rest.
    """
    from capsem.gate import cargotarget

    config = _capped(tmp_path, cap_gb=1.0)
    pulled = tmp_path / "pulled-bin"
    pulled.mkdir()
    (pulled / "capsem").write_text("#!/bin/sh\n", encoding="utf-8")
    prefix_path = tmp_path / "prefixes" / ("a" * 8)

    cargotarget.link_pulled_binaries(config, prefix_path, pulled)

    resolved = prefix_path / "target" / "debug" / "capsem"
    assert resolved.is_file(), "a hardcoded target/debug path must resolve to the pulled binary"
    assert (prefix_path / "target" / "debug").readlink() == pulled


def test_a_pulled_lane_refuses_to_read_binaries_it_built_itself(tmp_path: Path) -> None:
    """The point is the manifest's bytes, not whichever bytes are nearest."""
    from capsem.gate import cargotarget
    from capsem.gate.errors import GateError

    config = _capped(tmp_path, cap_gb=1.0)
    pulled = tmp_path / "pulled"
    pulled.mkdir()
    prefix_path = tmp_path / "prefixes" / ("b" * 8)
    (prefix_path / "target" / "debug").mkdir(parents=True)

    with pytest.raises(GateError, match="rather than the ones the manifest selected"):
        cargotarget.link_pulled_binaries(config, prefix_path, pulled)

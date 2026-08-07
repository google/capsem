"""What the gate actually did to the filesystem, asserted while it happens.

`contends` is a list somebody typed, and `can_overlap` compares two such lists
to each other. Nothing in that loop has ever looked at a disk, so a step that
does not mention what it touches satisfies every check by saying nothing --
and the writer is usually not the step at all but a unit test three
subprocesses down.

That gap cost two hours of release runs. `rust-coverage` runs the capsem-admin
suite, which builds release channels from the real `config/` tree and hardlinks
those checked-in files into `target/`. `linux-rust` is a container with the
same tree bind-mounted read-only over virtiofs. The scheduler ran them together
because their declarations were disjoint, and the container got an intermittent
`Permission denied` on a file that was `0644` before and `0644` after.

Two things follow, and each has tests here. The fault must be reported *as it
happens*, not left in a file for someone to analyze later. And the report must
carry what the fault is made of -- mode, size, inode, link count -- because
those are what distinguish a hardlinked source file from a copy, and a
flip-flop from a change.
"""

from __future__ import annotations

import os
import shutil
import time
from pathlib import Path

from capsem.gate import config as gate_config
from capsem.gate.faultlog import FaultLog
from capsem.gate.faults import Event, Facts, Fault
from capsem.gate.observation import Watch

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: From config, not spelled here: `test_gate_has_no_literal_data` holds the
#: gate's modules to one copy of every path, and a test that hardcodes its own
#: would be asserting against a value production no longer uses.
FD_PATH_TEMPLATE = gate_config.load(PROJECT_ROOT).runlog.fd_path_template


def _settle(watch: Watch, count: int, timeout: float = 5.0) -> None:
    """Wait for delivery rather than guessing with a sleep.

    FSEvents coalesces on its own schedule; a fixed sleep is either slow or
    flaky, and a test written because of a race may not introduce one.
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline and len(watch.events) < count:
        time.sleep(0.02)


def _until(predicate, timeout: float = 5.0) -> None:
    """Wait for the thing being asserted, not for a proxy of it.

    `_settle` waits on `watch.events`, and for an assertion about *faults* that
    is the wrong quantity by one line of `Watch.observed`: the event is
    appended and only then judged, both on the watchdog thread. A test polling
    the event count can therefore win the race into the gap between the two
    and assert on faults that are microseconds from existing.

    Measured on an unchanged tree, three runs in five failed that way -- and it
    surfaced as a red release gate at minute eight, which is an expensive place
    to learn that a test was watching the wrong variable.
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline and not predicate():
        time.sleep(0.02)


def _watch(root: Path, **kwargs) -> Watch:
    return Watch([root], source_root=root, **kwargs)


# ---------------------------------------------------------------------------
# Reported as it happens
# ---------------------------------------------------------------------------


def test_a_fault_is_emitted_when_it_happens_not_at_the_end(tmp_path: Path) -> None:
    """The difference between a check and a log.

    A rule evaluated after the run is acted on when someone thinks to read a
    file, which for a sixty-minute gate means the fault is found at minute
    sixty or never. This asserts the callback has already fired while the run
    is still going.
    """
    (tmp_path / "config").mkdir()
    seen: list[Fault] = []

    with _watch(tmp_path, on_fault=seen.append) as watch:
        watch.entered("suite")
        (tmp_path / "config" / "profile.toml").write_text("x", encoding="utf-8")
        _until(lambda: bool(seen))
        # Still inside the run: no sweep, no exit, no analysis pass.
        assert seen, "the fault was queued for later instead of raised now"
        assert seen[0].reason == "source-tree"
        watch.left("suite")


def test_the_error_log_survives_a_run_that_is_killed(tmp_path: Path) -> None:
    """Line-buffered and fsynced, because the run being described may not
    exit cleanly -- and that is when the report matters most."""
    log_path = tmp_path / "errors.log"
    log = FaultLog(log_path, max_bytes=1 << 20, keep=3)
    log(Fault(path=Path("/repo/config/x"), steps=("a",), reason="source-tree", detail="written"))

    # Read without closing: exactly what a killed run leaves behind.
    assert "source-tree" in log_path.read_text(encoding="utf-8")
    assert "/repo/config/x" in log_path.read_text(encoding="utf-8")
    log.close()


# ---------------------------------------------------------------------------
# What the fault is made of
# ---------------------------------------------------------------------------


def test_a_transient_mode_change_is_seen_even_though_it_reverts(tmp_path: Path) -> None:
    """The failure that started this. A mode dropped and restored inside one
    step leaves the file exactly as found, so any before/after comparison
    reports nothing happened -- while a concurrent reader gets `Permission
    denied` and the gate blames the environment."""
    (tmp_path / "config").mkdir()
    target = tmp_path / "config" / "seed.json"
    target.write_text("{}", encoding="utf-8")
    before = target.stat().st_mode & 0o777

    with _watch(tmp_path) as watch:
        watch.entered("suite")
        _settle(watch, 1)
        target.chmod(0o000)
        _settle(watch, 2)
        target.chmod(before)
        _settle(watch, 3)
        watch.left("suite")

    assert target.stat().st_mode & 0o777 == before, "the test must leave no trace"
    assert any(event.path == target for event in watch.events), (
        "a change that reverts within one step went unobserved"
    )
    # What is *guaranteed*: the file was touched, and touching checked-in
    # source during a run is itself the fault. Naming the intermediate mode is
    # best-effort and deliberately not asserted here -- FSEvents notifies and
    # we `stat` afterwards, so a mode restored inside that window is already
    # gone when we look. Claiming otherwise would be a guard that passes on a
    # fixture and misses the thing it was built for.
    assert any(fault.reason == "source-tree" for fault in watch.faults), (
        f"got {[f.reason for f in watch.faults]}"
    )


def test_a_mode_that_returns_to_a_previous_value_is_a_flip_flop() -> None:
    """The rule itself, driven directly.

    Live detection depends on winning a race against the restore, so the rule
    is proven here where the modes are known, and treated as opportunistic in
    the field.
    """
    watch = Watch([], source_root=Path("/repo"))
    path = Path("/repo/target/seed.json")
    for mode in (0o644, 0o000, 0o644):
        watch._judge(Event(at=1.0, kind="modified", path=path, steps=(), facts=Facts(mode=mode)))

    flip = [fault for fault in watch.faults if fault.reason == "mode-flip-flop"]
    assert flip, [fault.reason for fault in watch.faults]
    assert "0000 -> 0644" in flip[0].render()


def test_a_hardlink_into_build_output_is_named_where_it_lands(tmp_path: Path) -> None:
    """The actual bug, and the reason the first version of this module could
    not have found it.

    Hardlinking `config/x` to `target/y` creates a directory entry in
    `target/` and leaves `config/` untouched -- no event fires there, ever.
    Watching the source tree is structurally blind to it. The only trace is
    that the new file's inode is one a checked-in file already owns, which is
    decidable from one `stat` and needs no concurrency at all.
    """
    import subprocess

    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    (tmp_path / "config").mkdir()
    seed = tmp_path / "config" / "projects.json"
    seed.write_text("{}", encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)

    output = tmp_path / "target" / "release"
    output.mkdir(parents=True)
    staged = output / "root-payload-abc"

    live: list[Fault] = []
    with Watch([tmp_path / "target"], source_root=tmp_path, on_fault=live.append) as watch:
        watch.entered("contracts.release")
        os.link(seed, staged)
        _settle(watch, 1)
        watch.left("contracts.release")

    hardlink = [fault for fault in watch.faults if fault.reason == "hardlinked-source"]
    assert hardlink, (
        f"the staged payload shares an inode with checked-in source and went "
        f"unnamed; saw {[(f.reason, f.path.name) for f in watch.faults]}"
    )
    assert "config/projects.json" in hardlink[0].render()
    assert hardlink[0] in live, "found only after the fact, not while it happened"


def test_source_writable_beyond_its_owner_is_named() -> None:
    watch = Watch([], source_root=Path("/repo"))
    watch._judge(
        Event(
            at=1.0,
            kind="modified",
            path=Path("/repo/scripts/x.sh"),
            steps=(),
            facts=Facts(mode=0o666),
        )
    )
    assert "over-permission" in {fault.reason for fault in watch.faults}


def test_two_overlapping_steps_touching_one_build_path_is_named() -> None:
    """Build output, because a source path is the graver finding and is
    reported as that instead."""
    watch = Watch([], source_root=Path("/repo"), declared={"a": frozenset(), "b": frozenset()})
    watch._judge(
        Event(at=1.0, kind="modified", path=Path("/repo/target/store.db"), steps=("a", "b"))
    )
    assert "undeclared-contention" in {fault.reason for fault in watch.faults}


def test_a_declared_shared_resource_is_not_a_fault() -> None:
    """Otherwise every legitimately shared lane reds and the check gets muted,
    which is how a check stops being one."""
    watch = Watch(
        [],
        source_root=Path("/repo"),
        declared={"a": frozenset({"asset_tree"}), "b": frozenset({"asset_tree"})},
    )
    watch._judge(
        Event(at=1.0, kind="modified", path=Path("/repo/target/assets/x"), steps=("a", "b"))
    )
    assert watch.faults == []


def test_build_output_is_not_the_checked_in_tree() -> None:
    watch = Watch([], source_root=Path("/repo"))
    watch._judge(Event(at=1.0, kind="modified", path=Path("/repo/target/x"), steps=("build",)))
    assert watch.faults == []


def test_every_build_root_is_build_output_not_only_target() -> None:
    """`dist/`, `packages/` and `assets/` are gitignored and rewritten per run.

    Only `target` was excluded, so deleting a stale `.deb` or resyncing
    `assets/current` -- both ordinary steps -- read as the gate mutating the
    tree it is qualifying.
    """
    watch = Watch([], source_root=Path("/repo"))
    for directory in ("target", "dist", "packages", "assets", ".git", "node_modules", ".venv"):
        watch._judge(
            Event(at=1.0, kind="unlink", path=Path(f"/repo/{directory}/x"), steps=("build",))
        )
    assert watch.faults == [], [fault.render() for fault in watch.faults]


def test_a_relative_path_is_never_judged_against_the_working_directory(
    tmp_path: Path, monkeypatch
) -> None:
    """The release run of 2026-08-04 logged 42 of these, every one false.

    `shutil.rmtree` deletes through a directory descriptor --
    `os.unlink('profile.toml', dir_fd=5)` -- so a bare entry name reaches the
    observer. Resolving it against the current working directory, which is the
    checkout root, named a tracked file the run never touched:

        [source-tree] profile.toml: unlink during the run

    A guard that reports 42 phantom source mutations per run is a guard nobody
    reads, and this is the guard that exists to catch the `config/profiles`
    race that killed a release run. Judged only on absolute paths, so no
    caller's spelling can be misattributed -- not just the one that was found.
    """
    # cwd *is* the source root here. Anything less and this passes because
    # `Path.resolve()` happened to land outside the tree, which is the test
    # passing by coincidence rather than by the rule.
    watch = Watch([], source_root=tmp_path)
    monkeypatch.chdir(tmp_path)
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "profile.toml").write_text("x")

    watch._judge(Event(at=1.0, kind="unlink", path=Path("config/profile.toml"), steps=()))
    assert watch.faults == [], [fault.render() for fault in watch.faults]


def test_rmtree_of_build_output_is_not_reported_as_a_source_mutation(
    tmp_path: Path, monkeypatch
) -> None:
    """The production shape end to end, through the real interception."""
    from capsem.gate.interception import Instrument

    root = tmp_path / "checkout"
    (root / "target" / "config" / "profiles" / "code").mkdir(parents=True)
    (root / "config" / "profiles").mkdir(parents=True)
    (root / "target" / "config" / "profiles" / "code" / "profile.toml").write_text("x")
    (root / "target" / "config" / "profiles" / "code" / "asset-status.json").write_text("y")

    # cwd at the checkout root is what made a bare entry name resolve into the
    # source tree in the first place.
    monkeypatch.chdir(root)
    watch = Watch(roots=(root,), source_root=root)
    with Instrument(watch, fd_path_template=FD_PATH_TEMPLATE):
        shutil.rmtree(root / "target" / "config")

    offenders = [fault for fault in watch.faults if fault.reason == "source-tree"]
    assert not offenders, [fault.render() for fault in offenders]


def test_an_intercepted_fault_names_an_absolute_path(tmp_path: Path, monkeypatch) -> None:
    """A fault nobody can locate is not evidence.

    Separate from the rule above, because a fix that only widened the
    build-output set would silence the false positives and leave every real
    fault still reported as a bare basename.
    """
    from capsem.gate.interception import Instrument

    root = tmp_path / "checkout"
    (root / "config" / "profiles").mkdir(parents=True)
    victim = root / "config" / "profiles" / "profile.toml"
    victim.write_text("x")

    monkeypatch.chdir(root)
    watch = Watch(roots=(root,), source_root=root)
    with Instrument(watch, fd_path_template=FD_PATH_TEMPLATE):
        handle = os.open(str(root / "config" / "profiles"), os.O_RDONLY)
        try:
            os.unlink("profile.toml", dir_fd=handle)
        finally:
            os.close(handle)

    assert watch.faults, "a real tracked-source unlink went unreported"
    fault = watch.faults[0]
    assert fault.path.is_absolute(), f"fault names a bare path: {fault.render()}"
    assert fault.path == victim.resolve(), fault.render()


def test_an_empty_artifact_is_only_decidable_at_the_end(tmp_path: Path) -> None:
    """Mid-run it is a file being written; at the end it is a build that
    reported success and produced nothing."""
    target = tmp_path / "target"
    target.mkdir()
    artifact = target / "capsem.pkg"
    artifact.write_bytes(b"")

    watch = Watch([], source_root=tmp_path)
    watch.events.append(Event(at=1.0, kind="modified", path=artifact, steps=("package",)))
    assert "empty-artifact" in {fault.reason for fault in watch.sweep()}


def test_identical_bytes_under_two_names_are_named(tmp_path: Path) -> None:
    watch = Watch([], source_root=tmp_path)
    for name in ("one", "two"):
        watch._judge(
            Event(
                at=1.0,
                kind="modified",
                path=tmp_path / "target" / name,
                steps=(),
                facts=Facts(inode=1 if name == "one" else 2, digest="deadbeef"),
            )
        )
    assert "duplicate-content" in {fault.reason for fault in watch.faults}


# ---------------------------------------------------------------------------
# Observable by construction
# ---------------------------------------------------------------------------


def test_interception_sees_a_hardlink_with_no_watcher_at_all(tmp_path: Path) -> None:
    """No FSEvents, no coalescing, no notification latency, no polling.

    The call itself is the observation, so this holds for a path nobody
    thought to watch -- which is the whole point: the previous design could
    only see roots someone remembered to list.
    """
    import subprocess

    from capsem.gate.interception import CURRENT_STEP, Instrument

    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    (tmp_path / "config").mkdir()
    seed = tmp_path / "config" / "seed.json"
    seed.write_text("{}", encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)

    watch = Watch([], source_root=tmp_path)
    token = CURRENT_STEP.set("contracts.release")
    try:
        with Instrument(watch, fd_path_template=FD_PATH_TEMPLATE):
            (tmp_path / "target").mkdir()
            os.link(seed, tmp_path / "target" / "staged-payload")
    finally:
        CURRENT_STEP.reset(token)

    hardlink = [fault for fault in watch.faults if fault.reason == "hardlinked-source"]
    assert hardlink, [f.reason for f in watch.faults]
    assert hardlink[0].steps == ("contracts.release",), (
        "the caller is known exactly, not narrowed to whatever was in flight"
    )


def test_interception_catches_the_mode_that_a_watcher_arrives_too_late_for(
    tmp_path: Path,
) -> None:
    """The flip-flop, decided rather than raced for.

    An external watcher is notified and then stats, by which time the restore
    has happened and both samples read `0644`. Wrapping `chmod` has the old
    mode in hand before the call returns.
    """
    from capsem.gate.interception import Instrument

    target = tmp_path / "target" / "artifact"
    target.parent.mkdir()
    target.write_text("x", encoding="utf-8")
    target.chmod(0o644)

    watch = Watch([], source_root=tmp_path)
    with Instrument(watch, fd_path_template=FD_PATH_TEMPLATE):
        os.chmod(target, 0o000)
        os.chmod(target, 0o644)

    assert target.stat().st_mode & 0o777 == 0o644, "left exactly as found"
    assert "mode-flip-flop" in {fault.reason for fault in watch.faults}, [
        f.reason for f in watch.faults
    ]


def test_every_mutating_primitive_the_stdlib_offers_is_intercepted() -> None:
    """Read against the module, so the list cannot quietly fall behind.

    A primitive added to the codebase tomorrow and not added here is exactly
    how "observable by construction" decays back into "observable if someone
    remembered", which is the failure this replaced.
    """
    import shutil as _shutil

    from capsem.gate.interception import Instrument

    intercepted = {(module, name) for module, name, _ in Instrument.TARGETS}
    mutating = {
        (os, "link"),
        (os, "symlink"),
        (os, "unlink"),
        (os, "remove"),
        (os, "rmdir"),
        (os, "chmod"),
        (os, "rename"),
        (os, "replace"),
        (os, "truncate"),
        (_shutil, "copy"),
        (_shutil, "copy2"),
        (_shutil, "copyfile"),
        (_shutil, "copytree"),
        (_shutil, "rmtree"),
        (_shutil, "move"),
    }
    assert mutating <= intercepted, mutating - intercepted


def test_the_primitives_are_restored_afterwards() -> None:
    """A gate that leaves the standard library patched has broken every
    process that outlives it."""
    from capsem.gate.interception import Instrument

    before = (os.link, os.chmod, shutil.copytree)
    with Instrument(Watch([], source_root=Path("/repo")), fd_path_template=FD_PATH_TEMPLATE):
        assert os.link is not before[0], "not actually patched"
    assert (os.link, os.chmod, shutil.copytree) == before


def test_the_fault_log_is_bounded(tmp_path: Path) -> None:
    """A run that trips one rule per file trips it thousands of times.

    Unbounded, this is a disk-full outage wearing a helpful name -- which is
    why the cap is configured rather than assumed, and why the *newest*
    faults survive: they describe the failure being looked at.
    """
    log_path = tmp_path / "errors.log"
    log = FaultLog(log_path, max_bytes=512, keep=2)
    for index in range(400):
        log(Fault(path=Path(f"/repo/target/{index}"), steps=(), reason="x", detail="y" * 40))
    log.close()

    generations = sorted(tmp_path.glob("errors.log*"))
    assert len(generations) <= 3, generations
    total = sum(path.stat().st_size for path in generations)
    assert total <= 512 * 3, f"{total} bytes across {generations}"
    assert "/repo/target/399" in log_path.read_text(encoding="utf-8"), "newest fault was dropped"


def test_a_nested_ignored_tree_is_not_reported_as_source(tmp_path: Path) -> None:
    """`crates/capsem-app/gen/` is gitignored Tauri output, reported every run.

    The classifier compared `relative.parts[0]` against a hand-written set of
    build-output names, so nothing nested could ever match -- the first
    component is `crates`, and four faults per run named files the gate is
    right to create. Widening the set is whack-a-mole: the next generated
    directory lands somewhere else again.

    Git knows, and is asked once per `Watch`. Directories as well as files, so
    a path *created* under an ignored tree during the run is recognised too --
    which is precisely the case being reported, and one a snapshot of existing
    paths would miss.
    """
    import subprocess

    from capsem.gate.observation import Watch

    root = tmp_path / "checkout"
    (root / "crates" / "app").mkdir(parents=True)
    (root / "src").mkdir()
    (root / ".gitignore").write_text("crates/app/gen/\ntarget/\n", encoding="utf-8")
    (root / "src" / "real.py").write_text("x = 1\n", encoding="utf-8")
    for argv in (("init", "-q"), ("add", ".gitignore", "src/real.py")):
        subprocess.run(["git", *argv], cwd=root, check=True, capture_output=True)

    watch = Watch([root], source_root=root)

    # Created *after* the watch exists, which is the case that matters and the
    # one the first fix missed: `crates/capsem-app/gen/` is gitignored, so it
    # is not in the private copy at all until Tauri's build script makes it
    # mid-run. A list of ignored paths gathered at startup cannot contain it,
    # because it did not exist to be listed.
    (root / "crates" / "app" / "gen" / "schemas").mkdir(parents=True)
    generated = root / "crates" / "app" / "gen" / "schemas" / "acl.json"
    generated.write_text("{}\n", encoding="utf-8")
    assert not watch.is_source(generated), (
        "gitignored build output was classified as the source under test"
    )
    assert watch.is_source(root / "src" / "real.py"), "and real source still counts"
    # The hand-written names stay too: a fixture is not always a repository,
    # and git answers nothing outside one.
    assert not watch.is_source(root / "target" / "debug" / "x.bin")


def test_a_symlink_is_recorded_where_it_was_created(tmp_path: Path) -> None:
    """The link's own path, not the string it points at.

    `os.symlink(src, dst)` creates `dst`, like `link` and `copy` -- but
    `symlink` was missing from `DESTINATION_IS_SECOND`, so the *target* was
    recorded as though it were the created path. `Path.symlink_to("arm64")`
    passes a bare relative name, and `resolve()` anchored it to the checkout
    root, producing a report that `<root>/arm64` had been written: a path no
    step touched, not gitignored, and therefore judged to be source.

    Harmless while faults were only logged. Once a source-tree fault began
    aborting releases, it stopped one at `assets.assemble`.
    """
    from capsem.gate.interception import Instrument

    output = tmp_path / "target" / "assets"
    output.mkdir(parents=True)
    (output / "arm64").mkdir()

    with _watch(tmp_path) as watch, Instrument(watch, fd_path_template="/proc/self/fd/{fd}"):
        (output / "current").symlink_to("arm64")

    recorded = {event.path for event in watch.events}
    assert output / "current" in recorded, (
        f"the link's own path was not recorded; saw {sorted(map(str, recorded))}"
    )
    assert tmp_path / "arm64" not in recorded, (
        "the link target was resolved against the checkout root and recorded "
        "as a write to a path nothing touched"
    )

"""Citadel guard: reuse has to actually carry bytes, not merely be wired.

`test_build_cache_is_reusable.py` guards the mechanism -- a reclaimed prefix
hands its output back, lending never overwrites, only gitignored trees are lent.
Every one of those passed while the cache carried nothing at all, because none
of them asks the question the feature exists to answer: does the next run get
the last run's work?

That gap is not hypothetical. `[prefix] lent` was set to a directory chosen by
reasoning rather than measurement, and nothing would have failed if it had named
a path no run ever writes. The cache would have stayed empty and every run would
have stayed cold, with a green suite the whole way.

So these assert the effect end to end, against the real functions in the real
order: produce, reclaim, allocate, lend, read the bytes back.
"""

from __future__ import annotations

from pathlib import Path

from capsem_builder.gate import buildcache, prefix
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[2]

#: What an expensive tree costs to rebuild, standing in for a rootfs image.
PAYLOAD = b"an image that takes twenty minutes to build"


def _relocated(tmp_path: Path):
    """The real configuration, pointed at trees this test may destroy."""
    config = gate_config.load(ROOT)
    settings = config.prefix.model_copy(
        update={
            "parent": str(tmp_path / "prefixes"),
        }
    )
    return config.model_copy(update={"prefix": settings})


def _produced(config, name: str) -> Path:
    """A prefix holding what a finished run leaves behind."""
    path = Path(config.prefix.parent) / name
    for relative in config.prefix.lent:
        built = path / relative / "x86_64"
        built.mkdir(parents=True)
        (built / "rootfs.erofs").write_bytes(PAYLOAD)
    return path


def test_the_next_run_reads_back_what_the_last_run_built(tmp_path: Path) -> None:
    """The whole feature, in the order `_run_locked` performs it.

    A finished run reclaims its own prefix, which salvages; the run after it
    allocates a fresh prefix and is lent what the cache holds. If either half
    stops working the bytes are gone and every commit is cold again -- which is
    the state this was written to end, and the state it was actually in.
    """
    config = _relocated(tmp_path)
    finished = _produced(config, "aaaaaaaa")

    prefix.reclaim(config, finished)
    following = prefix.allocate(config, "bbbbbbbb")
    following.mkdir()
    lent = buildcache.lend(config, following)

    assert sorted(lent) == sorted(config.prefix.lent), (
        f"the next run was lent {lent} of {list(config.prefix.lent)}"
    )
    for relative in config.prefix.lent:
        carried = following / relative / "x86_64" / "rootfs.erofs"
        assert carried.is_file() and carried.read_bytes() == PAYLOAD, (
            f"{relative} did not survive the trip, so the next run rebuilds it"
        )


def test_a_lent_tree_is_one_the_gate_is_known_to_produce(tmp_path: Path) -> None:
    """A path no run writes is a cache that stays empty and a suite that stays green.

    Two lists establish that a run writes a tree. `[prefix] exports` is what
    must come back out before a prefix is reclaimed, which a release publishes
    from. `[prefix] produced` is build scaffolding a run writes and nothing
    publishes -- worth carrying between runs, and with no business being
    copied back into the checkout.

    Lending something in neither means lending something nothing is known to
    write, which is exactly how this was nearly configured.
    """
    settings = gate_config.load(ROOT).prefix

    known = set(settings.exports) | set(settings.produced)
    unproduced = sorted(set(settings.lent) - known)
    assert not unproduced, (
        f"{unproduced} is lent between runs but appears in neither "
        "`[prefix] exports` nor `[prefix] produced`, so nothing establishes "
        "that any run writes it"
    )


def test_scaffolding_is_never_copied_back_into_the_checkout() -> None:
    """The two provenance lists mean different things and must not overlap.

    An export lands in the developer's tree. Scaffolding that arrived there
    would be build output masquerading as source, which is the failure
    `[prefix] exports` is narrow to avoid.
    """
    settings = gate_config.load(ROOT).prefix
    both = sorted(set(settings.exports) & set(settings.produced))
    assert not both, f"{both} is declared as both published output and scaffolding"


def test_a_tree_reached_through_a_link_is_carried_as_its_contents(tmp_path: Path) -> None:
    """Profile content reaches the assets through a relative symlink.

    `target/ironbank-assets/<profile>/assets` is a link to `target/assets`,
    so what a run leaves behind is one real directory and several pointers at
    it. Moving a pointer instead of the directory would fill the cache with
    links into a prefix that no longer exists, and the next run would be lent a
    path that resolves to nothing.
    """
    config = _relocated(tmp_path)
    finished = _produced(config, "cccccccc")
    pointer = finished / "target" / "ironbank-assets" / "code"
    pointer.mkdir(parents=True)
    (pointer / "assets").symlink_to("../../assets")

    prefix.reclaim(config, finished)

    for relative in config.prefix.lent:
        cached = buildcache.root(config) / relative
        assert not cached.is_symlink(), f"the cache holds a link for {relative}, not a tree"
        assert (cached / "x86_64" / "rootfs.erofs").read_bytes() == PAYLOAD


class _Runner:
    """A run that builds the expensive tree and then finishes, or does not."""

    def __init__(self, status: int = 0) -> None:
        self.status = status
        self.notes: list[str] = []
        self.worked_in: list[Path] = []
        #: What each run found already built when it started. Recorded rather
        #: than read afterwards: a finished run reclaims its own prefix, so by
        #: the time the assertion runs there is nothing left to look at.
        self.found_on_entry: list[set[str]] = []

    def note(self, message: str) -> None:
        self.notes.append(message)

    def run(self, argv, *, cwd, env=None, check=True) -> int:
        self.worked_in.append(Path(cwd))
        self.found_on_entry.append(
            {
                relative
                for relative in _LENT
                if (Path(cwd) / relative / "x86_64" / "rootfs.erofs").is_file()
            }
        )
        for relative in _LENT:
            built = Path(cwd) / relative / "x86_64"
            built.mkdir(parents=True, exist_ok=True)
            (built / "rootfs.erofs").write_bytes(PAYLOAD)
        return self.status


_LENT: tuple[str, ...] = ()


def _sequenced(monkeypatch, config, tmp_path: Path, status: int = 0) -> _Runner:
    """One whole `_run_locked`, with only the source copy stubbed out.

    The copy is the one part that needs a real repository; everything this is
    about -- when the sweep runs, when the lend runs, when the salvage runs, and
    what the reclaim does on the way out -- is the real code in the real order.
    """
    global _LENT
    _LENT = config.prefix.lent
    monkeypatch.setattr(
        "capsem_builder.gate.snapshot.populate", lambda source, target, cfg: target.mkdir(parents=True)
    )
    monkeypatch.setattr("capsem_builder.gate.buildcache.export", lambda path, destination, cfg: None)
    return _Runner(status)


def _owning_its_checkout(config, tmp_path: Path):
    """The same config, pointed at a checkout the test owns.

    `_run_locked` adopts from `config.root`, and this repository's build trees
    are gigabytes. Six tests driving runs against the real checkout copied it
    six times and filled a 32 GiB tmpfs.
    """
    empty = tmp_path / "checkout"
    empty.mkdir(exist_ok=True)
    return config.model_copy(update={"root": empty})


def test_a_finished_run_leaves_its_work_for_the_one_after_it(tmp_path, monkeypatch) -> None:
    """The sequence that was wired and never demonstrated.

    Run one builds and completes; run two must start with what run one left.
    Every piece of this was unit-tested in isolation and the cache still held
    nothing after a night of real runs, because nothing asserted the pieces in
    the order `_run_locked` puts them.
    """
    config = _owning_its_checkout(_relocated(tmp_path), tmp_path)
    runner = _sequenced(monkeypatch, config, tmp_path)

    assert prefix.run_from_private_copy(runner, config, ["candidate"]) == 0
    assert buildcache.root(config).is_dir()

    second = _Runner(0)
    monkeypatch.setattr(
        "capsem_builder.gate.snapshot.populate",
        lambda source, target, cfg: target.mkdir(parents=True),
    )
    lent: list[str] = []
    original = buildcache.lend

    def watched(cfg, path):
        found = original(cfg, path)
        lent.extend(found)
        for relative in found:
            assert (path / relative / "x86_64" / "rootfs.erofs").read_bytes() == PAYLOAD
        return found

    monkeypatch.setattr("capsem_builder.gate.buildcache.lend", watched)
    assert prefix.run_from_private_copy(second, config, ["candidate"]) == 0

    assert sorted(lent) == sorted(config.prefix.lent), (
        f"the second run was lent {lent}; the first run's work was thrown away"
    )


def test_the_first_run_starts_from_what_the_checkout_already_holds(tmp_path, monkeypatch) -> None:
    """An empty cache is not the same as no previous work.

    `[prefix] exports` copies these trees back into the checkout at the end of
    every run, so the checkout holds the last completed run's output whether or
    not the cache does. Without this the cache only fills after a *finished*
    run, and the run that fills it pays the full cold cost -- which is what
    happened: the feature landed, several runs were killed or were source-only,
    and the cache sat empty through all of them while a 3.2 GiB tree the gate
    itself had exported sat in the checkout.

    Copied rather than moved. The checkout is the operator's, and `just shell`
    boots from that tree.
    """
    config = _relocated(tmp_path)
    checkout = tmp_path / "checkout"
    for relative in config.prefix.lent:
        built = checkout / relative / "x86_64"
        built.mkdir(parents=True)
        (built / "rootfs.erofs").write_bytes(PAYLOAD)

    adopted = buildcache.adopt(config, checkout)

    assert sorted(adopted) == sorted(config.prefix.lent)
    for relative in config.prefix.lent:
        assert (checkout / relative / "x86_64" / "rootfs.erofs").is_file(), (
            f"{relative} was taken from the checkout rather than copied"
        )
        assert (
            buildcache.root(config) / relative / "x86_64" / "rootfs.erofs"
        ).read_bytes() == PAYLOAD


def test_the_checkout_is_only_consulted_when_the_cache_is_empty(tmp_path) -> None:
    """What a run handed back beats what the checkout happens to hold.

    The cache holds the output of the most recent run; the checkout holds the
    most recent *export*, which is older whenever a run was killed before it
    exported. Preferring the checkout would quietly replace newer work with
    older, and content-addressed conditions would then rebuild it.
    """
    config = _relocated(tmp_path)
    checkout = tmp_path / "checkout"
    relative = config.prefix.lent[0]
    (checkout / relative).mkdir(parents=True)
    (checkout / relative / "older").write_bytes(b"stale")
    (buildcache.root(config) / relative).mkdir(parents=True)
    (buildcache.root(config) / relative / "newer").write_bytes(PAYLOAD)

    assert buildcache.adopt(config, checkout) == []
    assert (buildcache.root(config) / relative / "newer").read_bytes() == PAYLOAD
    assert not (buildcache.root(config) / relative / "older").exists()


def test_the_very_first_run_is_lent_what_the_checkout_exported(tmp_path, monkeypatch) -> None:
    """Wired, not merely available.

    `adopt` existing and never being called is this file's own subject one level
    along: a mechanism correct in isolation that reaches nothing. So this drives
    `_run_locked` with an empty cache and a checkout holding a previous export,
    and requires the run to start warm rather than build it all again.
    """
    config = _relocated(tmp_path)
    exported = tmp_path / "checkout"
    for relative in config.prefix.lent:
        built = exported / relative / "x86_64"
        built.mkdir(parents=True)
        (built / "rootfs.erofs").write_bytes(PAYLOAD)
    config = config.model_copy(update={"root": exported})

    runner = _sequenced(monkeypatch, config, tmp_path)
    received: list[Path] = []
    monkeypatch.setattr(
        "capsem_builder.gate.buildcache.export",
        lambda path, destination, cfg: received.append(path),
    )

    assert prefix.run_from_private_copy(runner, config, ["candidate"]) == 0

    assert runner.found_on_entry[0] == set(config.prefix.lent), (
        "the first run started cold while the checkout already held its output"
    )
    assert received, "the run never exported, so nothing would refresh the checkout"


def test_the_cache_never_holds_a_link_where_a_tree_should_be(tmp_path: Path) -> None:
    """A moved symlink points into a prefix that is about to stop existing.

    This happened. `~/.cg-build/assets` was a dangling link for most of a night:
    `du` reported it as a directory of zero bytes and `Path.exists()` followed it
    and answered False, so the cache read as populated to a person and as empty
    to the code. Every run then adopted the checkout again, and the one thing
    the cache is for -- carrying a finished run's work -- never happened once.

    A prefix legitimately reaches its assets through links; `[prefix] exports`
    says as much, and `target/ironbank-assets/<profile>/assets` is one. So the
    salvage has to move what a link points at, not the link.
    """
    config = _relocated(tmp_path)
    finished = Path(config.prefix.parent) / "dddddddd"
    for relative in config.prefix.lent:
        real = finished / "built" / relative / "x86_64"
        real.mkdir(parents=True)
        (real / "rootfs.erofs").write_bytes(PAYLOAD)
        link = finished / relative
        if relative in config.prefix.resumable:
            # A resumable receipt is an authority, not a selector. It is copied
            # so the prefix and cache both retain it, and a symlink is refused.
            (link / "x86_64").mkdir(parents=True)
            (link / "x86_64" / "rootfs.erofs").write_bytes(PAYLOAD)
            continue
        # A lent path may be nested -- `target/ironbank-assets` is -- so the
        # link needs its parent to exist, and a target that resolves from
        # where the link actually sits rather than from the prefix root.
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(real.parent)

    prefix.reclaim(config, finished)

    for relative in config.prefix.lent:
        cached = buildcache.root(config) / relative
        assert not cached.is_symlink(), (
            f"the cache holds a link for {relative}; it points into a prefix "
            "that has just been deleted, and reads as empty ever after"
        )
        assert (cached / "x86_64" / "rootfs.erofs").read_bytes() == PAYLOAD


def test_a_dangling_entry_never_passes_for_a_filled_cache(tmp_path: Path) -> None:
    """`exists()` follows links, so a broken one reads as nothing being there.

    The consequence is not that reuse fails loudly -- it is that every run
    quietly re-adopts and rebuilds while the cache appears to hold the tree.
    Whatever else is wrong, a lent path that is present but unusable has to be
    replaced rather than believed.
    """
    config = _relocated(tmp_path)
    relative = config.prefix.lent[0]
    cache = buildcache.root(config)
    cache.mkdir(parents=True)
    (cache / relative).parent.mkdir(parents=True, exist_ok=True)
    (cache / relative).symlink_to(tmp_path / "a-prefix-that-was-reclaimed")

    checkout = tmp_path / "checkout"
    built = checkout / relative / "x86_64"
    built.mkdir(parents=True)
    (built / "rootfs.erofs").write_bytes(PAYLOAD)

    assert buildcache.adopt(config, checkout) == [relative]
    assert not (cache / relative).is_symlink()
    assert (cache / relative / "x86_64" / "rootfs.erofs").read_bytes() == PAYLOAD


def test_adopting_never_reads_a_tree_the_run_was_not_pointed_at(tmp_path: Path) -> None:
    """The checkout it adopts from is the one the config names, and only that.

    Not a nicety: `adopt` copies whole build trees, and this repository's is
    3.2 GiB. A test that drives a run against the operator's real checkout
    copies it once per test -- six of them filled a 32 GiB tmpfs and every
    suite on the machine started failing on ENOSPC. Whatever `config.root`
    says is the only place this may read.
    """
    config = _relocated(tmp_path)
    elsewhere = tmp_path / "not-the-checkout"
    (elsewhere / config.prefix.lent[0]).mkdir(parents=True)
    (elsewhere / config.prefix.lent[0] / "rootfs.erofs").write_bytes(PAYLOAD)

    assert buildcache.adopt(config, tmp_path / "empty-checkout") == []
    assert not buildcache.root(config).exists() or not list(buildcache.root(config).iterdir())

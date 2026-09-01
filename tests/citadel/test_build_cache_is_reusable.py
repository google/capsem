"""Citadel guard: build output must outlive the prefix that produced it.

Every commit used to qualify cold. `just test` works in a private copy named
for the commit, so a fix on top of a qualified tree shares nothing with the run
before it -- three consecutive runs carried zero steps while a 42 GiB `cache/target/`
from the previous one sat on the same disk waiting for the next sweep to delete
it. `resume.py` had said so in its opening paragraph for months: "a fresh copy
per run starts with no `cache/target/`, so every replay is cold."

`buildcache` lends those trees to whichever prefix is running and takes them
back by `rename`. That is cheap and it is also sharp, so the two properties it
depends on are checked here rather than discovered during an hour-forty run:

  what is lent is invisible to the source digest, or a run qualifies a subject
  assembled from two commits

  a prefix gives its output back through every door it can leave by, or the
  first sweep deletes the thing this exists to keep
"""

from __future__ import annotations

import subprocess
from pathlib import Path, PurePosixPath

from capsem_builder.gate import buildcache, cargotarget, prefix
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[2]


def _config():
    return gate_config.load(ROOT)


def _relocated(tmp_path: Path):
    """The real configuration, pointed at trees this test may destroy."""
    config = _config()
    settings = config.prefix.model_copy(
        update={
            "parent": str(tmp_path / "prefixes"),
            "build_cache": str(tmp_path / "cache" / "target" / "prefix-products"),
            "vm_image_cache": str(tmp_path / "cache" / "target" / "assets" / "generations"),
            "cargo_target": str(tmp_path / "cache" / "target" / "cargo"),
        }
    )
    return config.model_copy(update={"prefix": settings})


def test_every_lent_path_is_invisible_to_the_source_digest() -> None:
    """Lending a tracked path would qualify a tree the operator never had.

    The subject is `git ls-files -co --exclude-standard`, so anything ignored
    is outside it by construction and anything else is inside it. Asked of git
    rather than restated as a second list here: the point is that the two
    definitions cannot drift, and a second list is exactly how they would.
    """
    lent = list(_config().prefix.lent)
    assert lent, "nothing is lent between runs, so every commit qualifies cold"

    ignored = subprocess.run(
        ["git", "check-ignore", "--stdin"],
        cwd=ROOT,
        input="\n".join(lent),
        capture_output=True,
        text=True,
        check=False,
    ).stdout.split()

    tracked = sorted(set(lent) - set(ignored))
    assert not tracked, (
        f"{tracked} is counted by the source digest and lent between runs, so "
        "a run would prove a tree assembled from more than one commit"
    )


def test_nothing_that_records_where_it_was_built_is_lent() -> None:
    """Cargo relocates a build directory. Build scripts do not.

    The first run handed a borrowed `cache/target/` died in Tauri's build script,
    which records its generated permission files by absolute `OUT_DIR` and went
    looking inside a prefix that had already been reclaimed. A uv venv is the
    same shape through `pyvenv.cfg` and its console-script shebangs, and a
    pnpm store links absolutely too.

    Named individually rather than inferred: there is no way to look at a
    directory and see whether something inside it wrote its own path down, so
    the list is the knowledge and this is where it is kept.

    Compiler output is reused by the other mechanism, which is why it is absent
    here rather than merely forgotten: `[prefix] cargo_target` gives every run
    one build directory at one absolute path, so the paths build scripts write
    down stay true instead of naming a prefix that no longer exists.
    """
    records_its_own_path = {"target", ".venv"}
    lent = set(_config().prefix.lent)

    overlap = sorted(lent & records_its_own_path)
    assert not overlap, (
        f"{overlap} contains artifacts that name the tree they were built in, "
        "so lending them to a differently named prefix produces paths nothing "
        "wrote; see the comment on `[prefix] lent`"
    )
    assert not any("node_modules" in relative for relative in lent), (
        "a pnpm store links absolutely, so a lent `node_modules` points into "
        "whichever prefix installed it"
    )


def test_the_cache_is_not_where_prefixes_are_swept(tmp_path: Path) -> None:
    """A cache under the prefix root survives exactly one run.

    `sweep` reclaims every directory under `parent` but the newest `keep`, and
    it recognizes a prefix by where it is rather than by what it is called.
    """
    config = _config()
    cache = buildcache.root(config).resolve()
    parent = prefix.parent_dir(config).resolve()
    assert parent != cache and parent not in cache.parents, (
        f"the lent build output at {cache} lives under the prefix root "
        f"{parent}, where the next sweep would reclaim it as a stale prefix"
    )


def test_a_prefix_gives_its_output_back_through_the_door_it_leaves_by(tmp_path: Path) -> None:
    """`reclaim` is that door -- a sweep, a repopulated release prefix, a
    successful run -- so the salvage belongs there and not at the call sites.

    Written against `reclaim` rather than against `salvage` on purpose. Calling
    the salvage directly proves the move works, which was never in doubt; what
    cost the rebuilds is a deletion path that does not call it.
    """
    config = _relocated(tmp_path)
    doomed = Path(config.prefix.parent) / "abcd1234"
    for relative in config.prefix.lent:
        (doomed / relative).mkdir(parents=True)
        (doomed / relative / "built").write_text("expensive", encoding="utf-8")

    prefix.reclaim(config, doomed)

    assert not doomed.exists()
    for relative in config.prefix.lent:
        recovered = buildcache.root(config) / relative / "built"
        assert recovered.read_text(encoding="utf-8") == "expensive", (
            f"{relative} was deleted with the prefix instead of salvaged, so "
            "the next commit rebuilds it from nothing"
        )


def test_lending_never_overwrites_what_the_prefix_already_built(tmp_path: Path) -> None:
    """A resumed prefix kept its own `cache/target/`, and it is the newer one.

    The cache can be holding an older tree from a run that was killed before it
    handed anything back. Overwriting here would replace the output a resume
    exists to reuse with the output it already superseded -- and `carry` would
    then accept a frontier proven by neither.
    """
    config = _relocated(tmp_path)
    relative = config.prefix.lent[0]
    working = Path(config.prefix.parent) / "abcd1234"
    (working / relative).mkdir(parents=True)
    (working / relative / "built").write_text("newer", encoding="utf-8")
    (buildcache.root(config) / relative).mkdir(parents=True)
    (buildcache.root(config) / relative / "built").write_text("older", encoding="utf-8")

    assert buildcache.lend(config, working) == []
    assert (working / relative / "built").read_text(encoding="utf-8") == "newer"


def test_the_public_complete_gate_never_discards_reusable_output() -> None:
    """Reuse is the complete local gate's default and cannot be opt-out.

    `just test` is the expensive whole-system proof. Quietly attaching the
    low-level cold diagnostic flag makes every forward fix rebuild everything
    and defeats the content-addressed cache this guard protects.
    """
    recipes = (ROOT / "justfile").read_text(encoding="utf-8")
    test_recipe = recipes.split("\ntest source_commit=", 1)[1].split("\n\n", 1)[0]
    assert "--clean-build" not in test_recipe, (
        "the public complete gate discards reusable build output; cold "
        "reproduction belongs to the explicit capsem-gate CLI flag"
    )


def test_an_over_threshold_compiler_cache_is_still_reused(tmp_path: Path) -> None:
    """The reuse contract covers policy as well as the public recipe spelling.

    The recipe guard above was green while a separate pre-run size policy
    deleted 41.9 GiB of compiler output at a 40 GiB threshold. Exercise that
    exact boundary: an advisory warning may become loud, but not destructive.
    """
    config = _relocated(tmp_path)
    settings = config.prefix.model_copy(update={"cargo_target_warning_gb": 0.000001})
    config = config.model_copy(update={"prefix": settings})
    shared = cargotarget.path(config)
    artifact = shared / "debug" / "deps" / "libcapsem.rlib"
    artifact.parent.mkdir(parents=True)
    payload = b"warm compiler output" * 128
    artifact.write_bytes(payload)

    observed = cargotarget.measure(config)

    assert observed.gb > config.prefix.cargo_target_warning_gb
    assert artifact.read_bytes() == payload, (
        "crossing the compiler-cache warning discarded the warm gate output; "
        "only an explicit --clean-build may do that"
    )


def test_compiler_output_is_shared_rather_than_lent() -> None:
    """The mechanism that replaces lending `cache/target/`, pinned where it is chosen.

    A shared build directory is only sound while it sits outside the prefix
    root -- inside it, `prefix.sweep` reclaims it as a prefix and the next run
    is cold again, which is the failure it exists to remove and the one nobody
    would look for.
    """
    prefix_config = _config().prefix
    shared = PurePosixPath(prefix_config.cargo_target)
    parent = PurePosixPath(prefix_config.parent)

    assert shared != parent and parent not in shared.parents, (
        f"{shared} is inside the prefix root, where a sweep reclaims it"
    )
    assert prefix_config.cargo_profiles, (
        "no profile directory is linked into the shared build root, so every "
        "checked-in `cache/target/cargo/debug/...` path resolves into the prefix instead"
    )
    assert "target" not in set(prefix_config.lent), (
        "compiler output is shared at one path, not lent between prefixes"
    )

"""Project shared compiler and immutable object views into a private prefix.

Split out of `prefix`, which had grown past the module ceiling this project
holds itself to -- and the seam is a real one rather than a line count.
`prefix` is about isolation: a private copy of the checkout, one per run, that
nobody can edit while it runs. This is the deliberate hole in that isolation,
the one directory every run deliberately shares, and it has its own rules about
what may be written, what may be deleted, and how large it may get.

Sharing is sound for the reason lending is not. Cargo records `OUT_DIR` under
whatever target directory it was given, so a build script's generated paths name
the prefix that produced them and the next run is a different prefix -- Tauri's
permission files hit exactly that. At one absolute path the baked-in paths stay
true, and cargo's fingerprints decide what is stale, which is the same judgement
a developer relies on when switching branches in one checkout.

The immutable object store is also machine-owned. Linking it here lets focused
builds and exact-source prefixes exchange verified component generations while
every mutable build, test, and runtime tree remains private to its run.

One writer, because one gate runs per machine under `flock`.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import NamedTuple

from . import cachelayout
from .config import GateConfig
from .errors import GateError

#: Bytes in a gibibyte, so the cap reads in the unit the config states it in.
_GIB = 1024**3


class Size(NamedTuple):
    """How large the shared build directory was."""

    gb: float


def path(config: GateConfig) -> Path:
    """The one build directory every run compiles into."""
    return cachelayout.shared_path(config, config.prefix.cargo_target)


def _generated_root(prefix: Path) -> Path:
    return prefix / "cache" / "target"


def link_pulled_binaries(config: GateConfig, prefix: Path, bin_dir: Path) -> None:
    """Put a release lane's pulled binaries where everything already looks.

    A pulled lane's binaries are staged outside the prefix, and roughly
    twenty-five checked-in test modules resolve a host binary as
    `PROJECT_ROOT/cache/target/cargo/debug/<name>`. Those paths are not wrong -- a test
    should not have to know that this run was handed its binaries instead of
    building them -- but in a prefix carrying only tracked files they name a
    directory nothing ever wrote.

    That cost three binary-release dispatches, each found one file at a time:
    the service and gateway helpers, then the CLI suite, with `--maxfail=5`
    hiding however many were behind them. Editing every call site is the same
    fix applied twenty-five times and forgotten on the twenty-sixth; a link is
    the mechanism, and it is the same one the compiler output already uses.
    """
    root = _generated_root(prefix) / "cargo"
    root.mkdir(parents=True, exist_ok=True)
    link = root / config.modules.default_bin_dir.rsplit("/", 1)[-1]
    if link.is_symlink():
        if link.readlink() == bin_dir:
            return
        link.unlink()
    elif link.exists():
        raise GateError(
            f"{link} is a real directory, so this lane would read binaries it "
            "built rather than the ones the manifest selected"
        )
    link.symlink_to(bin_dir, target_is_directory=True)


def link_profiles(config: GateConfig, prefix: Path) -> None:
    """Point this prefix's profile directories at the shared build root.

    Cargo is told where to write by `CARGO_TARGET_DIR`; these symlinks are for
    everything else. Roughly thirty checked-in paths name `cache/target/cargo/debug/...` or
    `cache/target/cargo/release/...` relative to the tree a step runs in, and they are
    correct -- a step should not have to know that compiler output is a
    property of the machine rather than of the run.

    Only the profile directories. The rest of `cache/target/` is the run's own: the
    journal it is writing, the config it materialized, the homes its VMs boot
    from. Sharing those would make two runs one run.
    """
    shared = path(config)
    root = _generated_root(prefix) / "cargo"
    root.mkdir(parents=True, exist_ok=True)
    for profile in config.prefix.cargo_profiles:
        (shared / profile).mkdir(parents=True, exist_ok=True)
        link = root / profile
        # A resumed prefix already has the link, and a populated one cannot:
        # `snapshot` copies tracked files, and `cache/target/` is gitignored.
        if link.is_symlink():
            if link.readlink() == shared / profile:
                continue
            link.unlink()
        elif link.exists():
            raise GateError(
                f"{link} is a real directory, so this run would compile into the "
                "prefix instead of the shared build root and pay a cold build"
            )
        link.symlink_to(shared / profile, target_is_directory=True)


def link_object_store(config: GateConfig, prefix: Path) -> None:
    """Give one private prefix the machine's immutable object authority."""
    paths = cachelayout.cache_paths(config)
    shared = paths.stage("objects")
    relative = paths.policy.root / paths.policy.stages["objects"].path
    link = prefix / relative
    shared.mkdir(parents=True, exist_ok=True)
    link.parent.mkdir(parents=True, exist_ok=True)
    if link.is_symlink():
        if link.readlink() == shared:
            return
        link.unlink()
    elif link.exists():
        raise GateError(
            f"{link} is a private object store, so this run would rebuild "
            "verified components instead of using the shared authority"
        )
    link.symlink_to(shared, target_is_directory=True)


def link_prefix_trees(config: GateConfig, prefix: Path) -> None:
    """Decide what this prefix's `cache/target/` points at, once, in one place.

    Two lanes and one answer. Ordinary runs compile into the shared build root;
    a release lane reads binaries and config a manifest selected and something
    else staged. Either way the checked-in tests resolve
    `PROJECT_ROOT/cache/target/...` and are right to, so the prefix is what makes that
    resolve to the correct tree.
    """
    link_object_store(config, prefix)
    pulled = os.environ.get(config.modules.release_bin_dir)
    if not pulled:
        link_profiles(config, prefix)
        return
    link_pulled_binaries(config, prefix, Path(pulled).resolve())
    # The materialized config the lane staged, taken from the checkout this
    # prefix is being made from rather than from an environment variable.
    # `CAPSEM_PROFILES_DIR` is an overlay the gate adds per step, so it is not
    # set when the prefix is built -- keying on it meant this never fired, and
    # a test that set it first agreed with the assumption instead of checking
    # it.
    staged = _generated_root(config.root) / "config"
    if staged.is_dir():
        link_pulled_tree(config, prefix, "config", staged.resolve())


def link_pulled_tree(config: GateConfig, prefix: Path, relative: str, target: Path) -> None:
    """Point one path under the prefix's `cache/target/` at a tree staged outside it.

    The same fix as the binaries and for the same reason. A release lane
    qualifies from a prefix carrying only tracked files, while the checked-in
    tests resolve `PROJECT_ROOT/cache/target/<something>` because a test should not
    have to know whether this run built its inputs or was handed them. Linking
    is what makes both true at once.

    Refuses a real directory rather than preferring it: a lane that exists to
    prove manifest-selected content must not quietly read content it made.
    """
    link = _generated_root(prefix) / relative
    link.parent.mkdir(parents=True, exist_ok=True)
    if link.is_symlink():
        if link.readlink() == target:
            return
        link.unlink()
    elif link.exists():
        raise GateError(
            f"{link} is a real directory, so this lane would read content it "
            "produced rather than the content the manifest selected"
        )
    link.symlink_to(target, target_is_directory=True)


def _tree_bytes(root: Path) -> int:
    """Add up the regular files under `root`, following nothing.

    `du` in Python rather than a subprocess because this runs on the way into
    every run and the answer is reported whether or not it triggers anything.
    Symlinks are not followed and not counted: every prefix points *into* this
    tree, and counting through them would bill the same bytes once per run.
    """
    total = 0
    stack = [root]
    while stack:
        try:
            entries = list(os.scandir(stack.pop()))
        except OSError:
            continue
        for entry in entries:
            try:
                if entry.is_dir(follow_symlinks=False):
                    stack.append(Path(entry.path))
                elif entry.is_file(follow_symlinks=False):
                    total += entry.stat(follow_symlinks=False).st_size
            except OSError:
                continue
    return total


def measure(config: GateConfig) -> Size:
    """Measure the shared build directory without changing it.

    The one part of this system nothing reclaimed. Prefixes are swept to
    `[prefix] keep` on the way into every run and the build cache is a rename
    target that never accumulates -- but cargo never garbage-collects, so every
    dependency bump and deleted crate leaves output here forever. It reached
    8 GB in three days.

    That growth is reported against an advisory threshold, but a normal gate
    never reclaims it. Selective deletion underneath Cargo corrupts its
    fingerprint judgement, while whole-directory deletion silently turns the
    expensive public qualification into a cold build. The operator may still
    request that deliberately with `--clean-build`; `[disk] required_free_gb`
    remains the fail-closed filesystem backstop.
    """
    shared = path(config)
    if not shared.is_dir():
        return Size(0.0)
    return Size(_tree_bytes(shared) / _GIB)

"""What a prefix's `target/` points at: the shared build root, or a pulled tree.

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

One writer, because one gate runs per machine under `flock`.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path
from typing import NamedTuple

from .config import GateConfig
from .errors import GateError

#: Bytes in a gibibyte, so the cap reads in the unit the config states it in.
_GIB = 1024**3


class Size(NamedTuple):
    """How large the shared build directory was, and whether it survived."""

    gb: float
    discarded: bool


def path(config: GateConfig) -> Path:
    """The one build directory every run compiles into."""
    return Path(config.prefix.cargo_target.format(parent=config.prefix.parent)).expanduser()


def link_pulled_binaries(config: GateConfig, prefix: Path, bin_dir: Path) -> None:
    """Put a release lane's pulled binaries where everything already looks.

    A pulled lane's binaries are staged outside the prefix, and roughly
    twenty-five checked-in test modules resolve a host binary as
    `PROJECT_ROOT/target/debug/<name>`. Those paths are not wrong -- a test
    should not have to know that this run was handed its binaries instead of
    building them -- but in a prefix carrying only tracked files they name a
    directory nothing ever wrote.

    That cost three binary-release dispatches, each found one file at a time:
    the service and gateway helpers, then the CLI suite, with `--maxfail=5`
    hiding however many were behind them. Editing every call site is the same
    fix applied twenty-five times and forgotten on the twenty-sixth; a link is
    the mechanism, and it is the same one the compiler output already uses.
    """
    root = prefix / "target"
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
    everything else. Roughly thirty checked-in paths name `target/debug/...` or
    `target/release/...` relative to the tree a step runs in, and they are
    correct -- a step should not have to know that compiler output is a
    property of the machine rather than of the run.

    Only the profile directories. The rest of `target/` is the run's own: the
    journal it is writing, the config it materialized, the homes its VMs boot
    from. Sharing those would make two runs one run.
    """
    shared = path(config)
    root = prefix / "target"
    root.mkdir(parents=True, exist_ok=True)
    for profile in config.prefix.cargo_profiles:
        (shared / profile).mkdir(parents=True, exist_ok=True)
        link = root / profile
        # A resumed prefix already has the link, and a populated one cannot:
        # `snapshot` copies tracked files, and `target/` is gitignored.
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


def link_prefix_trees(config: GateConfig, prefix: Path) -> None:
    """Decide what this prefix's `target/` points at, once, in one place.

    Two lanes and one answer. Ordinary runs compile into the shared build root;
    a release lane reads binaries and config a manifest selected and something
    else staged. Either way the checked-in tests resolve
    `PROJECT_ROOT/target/...` and are right to, so the prefix is what makes that
    resolve to the correct tree.
    """
    pulled = os.environ.get(config.modules.release_bin_dir)
    if not pulled:
        link_profiles(config, prefix)
        return
    link_pulled_binaries(config, prefix, Path(pulled).resolve())
    staged = os.environ.get(config.environment.profiles_dir)
    if staged:
        link_pulled_tree(config, prefix, "config", Path(staged).resolve().parent)


def link_pulled_tree(config: GateConfig, prefix: Path, relative: str, target: Path) -> None:
    """Point one path under the prefix's `target/` at a tree staged outside it.

    The same fix as the binaries and for the same reason. A release lane
    qualifies from a prefix carrying only tracked files, while the checked-in
    tests resolve `PROJECT_ROOT/target/<something>` because a test should not
    have to know whether this run built its inputs or was handed them. Linking
    is what makes both true at once.

    Refuses a real directory rather than preferring it: a lane that exists to
    prove manifest-selected content must not quietly read content it made.
    """
    link = prefix / "target" / relative
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


def bound(config: GateConfig) -> Size:
    """Measure the shared build directory, and discard it if it is over cap.

    The one part of this system nothing reclaimed. Prefixes are swept to
    `[prefix] keep` on the way into every run and the build cache is a rename
    target that never accumulates -- but cargo never garbage-collects, so every
    dependency bump and deleted crate leaves output here forever. It reached
    8 GB in three days.

    `[disk] required_free_gb` does not bound it and was never going to. That is
    a floor on the whole filesystem: it reports that something has *already*
    eaten the disk, and it stops whoever runs next rather than whatever grew. A
    cap on the directory is what keeps the floor from ever being the mechanism.

    Whole-directory, never selective. Cargo decides what is stale, by
    fingerprint, and deleting chosen files underneath it corrupts exactly that
    judgement -- the same reason `[prefix] lent` may only carry
    content-addressed output. So the price of the cap is one cold build,
    charged at a predictable size instead of at a full disk.
    """
    shared = path(config)
    if not shared.is_dir():
        return Size(0.0, False)
    gb = _tree_bytes(shared) / _GIB
    if gb <= config.prefix.cargo_target_max_gb:
        return Size(gb, False)
    # The same shape as `prefix.reclaim`: a recursive delete of a path
    # assembled in Python states out loud what it is allowed to be. Here that
    # is the configured build root and nothing else -- not the prefix parent
    # beside it, not a filesystem root, not the checkout.
    resolved = Path(os.path.abspath(shared))
    if resolved.parent == resolved or resolved == config.root.resolve():
        raise GateError(f"refusing to discard {resolved}: that is not a build directory")
    shutil.rmtree(resolved, ignore_errors=True)
    if resolved.exists():
        raise GateError(f"could not discard {resolved}; it is still on disk")
    return Size(gb, True)

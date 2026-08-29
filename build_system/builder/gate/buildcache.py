"""What a run's output does when the run ends.

A run works in a private prefix and reclaims it afterwards, which is the whole
isolation grant -- and it is also why `resume.py` opens by saying that "a fresh
copy per run starts with no `target/`, so every replay is cold". That was true
of far more than replays. Every new commit paid a full cold qualification: the
prefix is named for the commit, so a fix on top of a qualified tree shares
nothing with the run before it. Three consecutive runs carried zero steps while
a 42 GiB `target/` from the previous one sat on the same disk, waiting to be
deleted by the next sweep.

Three things happen to that output, and they are one subject: `export` brings
what a release publishes back into the checkout, `salvage` keeps the expensive
trees for the next run, and `lend` hands them over. All three run where `prefix`
runs -- before the journal exists and outside the machine lock.

The fix for the cold rebuilds is to stop treating build output as something a
prefix owns. It is a
property of the *machine* -- one gate runs at a time, enforced by `flock`, so
there is exactly one writer and no contention to arbitrate. The cache lends the
outputs to whichever prefix is about to run and takes them back when it ends,
by `rename`, so expensive trees are never copied and no two trees ever alias
the same inode. The one exception is a config-declared tiny receipt: it is
copied into the cache so a retained prefix keeps the authority that pins its
exact Docker products while a newer run borrows the warm copy. A hardlinked or
reflinked seed was considered first and is worse on both counts: this
filesystem is ext4, so there is no reflink to fall back on, and a hardlinked
`target/` lets an in-place write inside one prefix silently rewrite the retained
tree another run is meant to resume into.

Two rules keep it honest, and both are guarded:

  only gitignored paths may be lent. Anything the source digest counts is the
  subject under qualification, and a subject that survived from an earlier
  commit is not the tree the operator asked to prove

  every reclaim salvages first. `prefix.reclaim` is the single door a prefix
  leaves by -- sweep, a repopulated release prefix, a successful run -- so
  putting the salvage there means no new call site can forget it

Reuse is the default because a cold hour-forty per commit is the thing being
fixed. `--clean-build` discards the cache first, which is the escape hatch for
the one failure this trades against: a step that reads a file it did not
produce this run passes locally on stale output and fails in CI, where every
runner is cold.
"""

from __future__ import annotations

import errno
import shutil
from pathlib import Path

from . import cargotarget
from .config import GateConfig
from .errors import GateError
from .filesystem import copy_tree, merge_tree, remove


def export(prefix: Path, destination: Path, config: GateConfig) -> None:
    """Bring back what the run produced, before the copy is reclaimed.

    Anything built inside the prefix and not named in `[prefix] exports` dies
    with it. `packages/` is the one that matters most: the signed `.pkg` a
    release publishes is built inside the run, so omitting it is a gate that
    passes with nothing to ship.
    """
    exact_trees = {config.functional.assets_dir, config.functional.config_root}
    for relative in config.prefix.exports:
        origin = prefix / relative
        if not origin.exists():
            continue
        # A link *out* of the prefix names input, not output. A release lane
        # points `target/config` at the cohort it was handed, and copying that
        # back would export an input as though the run had produced it -- and
        # dies outright if the tree it names has since gone. A link *within*
        # the prefix is the local gate's own profile selector and must still be
        # dereferenced, which is what the assets case below is about.
        if origin.is_symlink() and not origin.resolve().is_relative_to(prefix.resolve()):
            continue
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if origin.is_dir():
            if relative in exact_trees:
                remove(target)
                # Follow a top-level profile selector, but retain selectors
                # inside the exported tree such as target/assets/current. The latter
                # is a relative link in the tree and materializing it copies a
                # multi-gigabyte architecture for no new bytes.
                copy_tree(origin, target)
            else:
                merge_tree(origin, target)
        else:
            shutil.copy2(origin, target)


def root(config: GateConfig) -> Path:
    """Where lent output lives between runs, beside the prefix root."""
    return Path(config.prefix.build_cache.format(parent=config.prefix.parent)).expanduser()


def _move(origin: Path, destination: Path) -> None:
    """Relocate a tree, or say plainly why it could not be relocated.

    `shutil.move` is deliberately not used. It falls back to a recursive copy
    across filesystems, which for `target/` is tens of gigabytes spent silently
    on every run -- the exact cost this module exists to avoid. A cache on the
    wrong filesystem should be a sentence the operator reads once, not a
    permanent tax nobody attributes.
    """
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        origin.rename(destination)
    except OSError as error:
        if error.errno != errno.EXDEV:
            raise
        raise GateError(
            f"cannot lend {origin} to {destination}: they are on different "
            "filesystems, so the move would become a full copy. Point "
            "`[prefix] build_cache` at the filesystem holding `[prefix] parent`."
        ) from error


def lend(config: GateConfig, prefix_path: Path) -> list[str]:
    """Give a prefix the machine's build output, and say what it received.

    Anything the prefix already holds is left alone. That is what makes a
    resumed prefix work unchanged: it kept its own `target/` when the run that
    filled it failed, and the cache must not overwrite the newer tree with an
    older one it happens to be holding.
    """
    cache = root(config)
    lent: list[str] = []
    for relative in config.prefix.lent:
        origin, destination = cache / relative, prefix_path / relative
        if not origin.exists() or destination.exists():
            continue
        _move(origin, destination)
        lent.append(relative)
    return lent


def salvage(config: GateConfig, prefix_path: Path) -> list[str]:
    """Take the build output back before the prefix is gone.

    Called on the way out of a run and again from `prefix.reclaim`, which is
    every way a prefix ends. The second is not redundant: a run that is killed
    between the two leaves its output in a tree the next sweep would delete,
    and salvaging at the door recovers it instead.

    A cache entry that is already there wins. It is the one this machine lent
    out and has not been given back, so the prefix's copy is either the same
    tree or an older one from a run that never returned it.

    Config-declared resumable authorities are copied, not moved. They are tiny
    receipts rather than build products, and the retained prefix must keep its
    exact receipt so storage reclamation cannot delete images needed to resume
    it after the shared copy has moved into a newer prefix.
    """
    cache = root(config)
    taken: list[str] = []
    for relative in config.prefix.lent:
        origin, destination = prefix_path / relative, cache / relative
        if not origin.exists() or destination.exists():
            continue
        if relative in config.prefix.resumable:
            if origin.is_symlink():
                raise GateError(f"resumable cache authority {origin} must not be a symlink")
            destination.parent.mkdir(parents=True, exist_ok=True)
            if origin.is_dir():
                copy_tree(origin, destination)
            else:
                shutil.copy2(origin, destination)
            taken.append(relative)
            continue
        # What the link points at, never the link. A prefix reaches its assets
        # through links -- `target/ironbank-assets/<profile>/assets` is one, and
        # `[prefix] exports` says a top-level selector may be another -- and a
        # moved link points into a prefix that is about to be deleted. The
        # result reads as a directory of zero bytes to `du` and as absent to
        # `Path.exists()`, which is how this cache spent a night looking full
        # and behaving empty.
        if origin.is_symlink():
            _move(origin.resolve(), destination)
            origin.unlink()
        else:
            _move(origin, destination)
        taken.append(relative)
    return taken


def adopt(config: GateConfig, checkout: Path) -> list[str]:
    """Fill an empty cache from the checkout, and say what it took.

    An empty cache is not the same as no previous work. `[prefix] exports`
    copies these trees back into the checkout at the end of every run, so the
    checkout holds the last completed run's output whether or not the cache
    does -- and without this the cache only fills after a *finished* run, while
    the run that fills it pays the full cold cost.

    That is not hypothetical. This landed, several runs in a row were killed or
    were source-only, and the cache sat empty through all of them while a
    3.2 GiB tree the gate itself had exported sat in the checkout.

    Copied rather than moved, unlike everywhere else here: the checkout is the
    operator's, `just shell` boots from that tree, and taking it away to save a
    copy would be the gate helping itself to something it does not own. Skipped
    entirely when the cache already holds a tree, because that one came from a
    run and the checkout's came from an export that may be older.
    """
    cache = root(config)
    adopted: list[str] = []
    for relative in config.prefix.lent:
        origin, destination = checkout / relative, cache / relative
        if not origin.is_dir() or destination.exists():
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        copy_tree(origin, destination)
        adopted.append(relative)
    return adopted


def discard(config: GateConfig) -> None:
    """Throw the retained output away, so the next run builds from nothing.

    Both halves of it. The lent trees are one, and the shared build directory
    is the other and much the larger -- a `--clean-build` that left the
    compiler output in place would be the flag that most needs to mean what it
    says meaning almost nothing.
    """
    remove(root(config))
    remove(cargotarget.path(config))

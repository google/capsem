"""Build output that outlives the private copy it was produced in.

A run works in a private prefix and reclaims it afterwards, which is the whole
isolation grant -- and it is also why `resume.py` opens by saying that "a fresh
copy per run starts with no `target/`, so every replay is cold". That was true
of far more than replays. Every new commit paid a full cold qualification: the
prefix is named for the commit, so a fix on top of a qualified tree shares
nothing with the run before it. Three consecutive runs carried zero steps while
a 42 GiB `target/` from the previous one sat on the same disk, waiting to be
deleted by the next sweep.

The fix is to stop treating build output as something a prefix owns. It is a
property of the *machine* -- one gate runs at a time, enforced by `flock`, so
there is exactly one writer and no contention to arbitrate. The cache lends the
outputs to whichever prefix is about to run and takes them back when it ends,
by `rename`, so nothing is ever copied and no two trees ever alias the same
inode. A hardlinked or reflinked seed was considered first and is worse on both
counts: this filesystem is ext4, so there is no reflink to fall back on, and a
hardlinked `target/` lets an in-place write inside one prefix silently rewrite
the retained tree another run is meant to resume into.

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
from pathlib import Path

from .config import GateConfig
from .errors import GateError
from .filesystem import remove


def root(config: GateConfig) -> Path:
    """Where lent output lives between runs."""
    return Path(config.prefix.build_cache).expanduser()


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
    """
    cache = root(config)
    taken: list[str] = []
    for relative in config.prefix.lent:
        origin, destination = prefix_path / relative, cache / relative
        if not origin.exists() or destination.exists():
            continue
        _move(origin, destination)
        taken.append(relative)
    return taken


def discard(config: GateConfig) -> None:
    """Throw the cache away, so the next run builds everything from nothing."""
    remove(root(config))

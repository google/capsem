"""A private copy of the checkout, one per run.

Every other isolation the gate has is a declaration checked against another
declaration. This one is the grant itself: a run reads a tree that nobody else
has a path to, so an outside edit cannot reach it and there is nothing left to
detect.

That distinction is the whole reason this exists. The observer already sees an
intruding write -- on the run that died at `source.verify` after 61 minutes it
flagged the first one at 22:15:56 and named the file at 22:21:27, 23 minutes
before the run stopped. Detection that early still cost the hour, because the
tree under the gate had already moved. Copying it cannot.

Two things are easy to get wrong here and both are expensive:

  the copy is the *working tree*, not `HEAD`. The source digest is
  `git ls-files -co --exclude-standard`, so uncommitted edits and untracked
  non-ignored files are part of the subject the gate qualifies, and a copy
  built from `HEAD` would qualify a different tree than the one measured

  `git ls-files` cannot see everything the run needs. `private/` is gitignored
  and holds the Tauri signing keys; `.git` is not a file at all. Both are
  declared in `[prefix] carried`, because the failure mode is a package lane
  that loses its key during a release rather than here
"""

from __future__ import annotations

import os
import secrets
import shutil
from pathlib import Path

from . import snapshot
from .config import GateConfig
from .errors import GateError
from .filesystem import copy_tree, merge_tree, remove

#: `cp` flags that ask APFS for copy-on-write. Clonefile is what makes this
#: cheap enough to do unconditionally -- 2074 files and `.git` measured 2.2s
#: and 186 MB, against 84s and 164 GB for a copy that also took `target/`.
#:
#: It also has to be copy-on-write rather than a hardlink. A hardlinked copy
#: satisfies every other property here and still lets an outside edit reach
#: into a running gate, which is the one thing this module exists to prevent.
_CLONE = ("-c",)

#: How many paths to hand one `cp`. Per-file invocation is 2080 subprocesses;
#: one invocation is an argv the kernel refuses.
_BATCH = 200


def parent_dir(config: GateConfig) -> Path:
    """Where prefixes live, with `~` resolved."""
    return Path(config.prefix.parent).expanduser()


def example(config: GateConfig) -> Path:
    """A representative prefix path, for arithmetic that must not boot a VM.

    The identity is random per run, so its *length* is the only part a budget
    can be checked against ahead of time.
    """
    return parent_dir(config) / ("0" * config.prefix.name_length)


def socket_root(config: GateConfig) -> Path:
    """The directory the gateway's AF_UNIX paths are built under.

    Absolute, and deliberately not inside the checkout -- so moving the run
    into a prefix cannot lengthen a socket path by even one byte. That is the
    property worth holding, and it is not obvious: the workspace run dir is
    relative to the root and would grow with it, and at
    `<root>/target/test-home/.capsem/run` it is already past `SUN_LEN` once
    the gateway's 54-byte suffix is added. The asset lane exists at
    `/tmp/capsem-a.XXXXXX` precisely so that number is never the binding one.
    """
    return Path(config.assets.run_dir_template).parent


def sweep(config: GateConfig) -> list[Path]:
    """Reclaim all but the newest `keep` prefixes, and say which went.

    On the way in rather than on the way out, for the reason `Workspace` wipes
    its home on entry: a run that failed leaves its tree so it can be inspected
    or resumed into, and the run *after* it is the one that no longer needs it.

    Nothing else does this. `[disk] reclaimable` only accepts paths inside the
    checkout, so before this a failed gate left its copy indefinitely -- 22 GiB
    on this machine, carrying the copied signing material with it.
    """
    root = parent_dir(config)
    if not root.is_dir():
        return []
    kept = sorted((p for p in root.iterdir() if p.is_dir()), key=lambda p: p.stat().st_mtime)
    stale = kept[: max(len(kept) - config.prefix.keep, 0)]
    for path in stale:
        reclaim(config, path)
    return stale


def allocate(config: GateConfig, identity: str) -> Path:
    """Reserve a named prefix, failing if something already holds the name."""
    root = parent_dir(config)
    root.mkdir(parents=True, exist_ok=True)
    path = root / identity[: config.prefix.name_length]
    if path.exists():
        raise GateError(f"prefix {path} already exists, so this run has no private tree")
    return path


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
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if origin.is_dir():
            if relative in exact_trees:
                remove(target)
                # Follow a top-level profile selector, but retain selectors
                # inside the exported tree such as assets/current. The latter
                # is a relative link in the tree and materializing it copies a
                # multi-gigabyte architecture for no new bytes.
                copy_tree(origin, target, symlinks=True)
            else:
                merge_tree(origin, target)
        else:
            shutil.copy2(origin, target)


def source_checkout(config: GateConfig) -> Path | None:
    """The real checkout, when this process is already running inside a prefix.

    Absent means this is the outer process and it should build one. Present
    means it must not, or the re-exec recurses forever -- and it also answers
    the question a prefixed run still has to be able to answer, which is which
    tree it was copied from.
    """
    value = os.environ.get(config.environment.source_checkout)
    return Path(value) if value else None


def run_from_private_copy(
    runner, config: GateConfig, arguments: list[str], *, reuse: Path | None = None
) -> int:
    """Copy the checkout, run the same command inside the copy, bring back what
    it produced, and give the copy back.

    Through `uv run` in the prefix rather than this interpreter, deliberately.
    Re-execing `sys.executable -m capsem.gate` would keep the *parent's*
    `sys.path`, so the child would run the outer checkout's code while sitting
    in the copy -- measuring one tree and qualifying another, which is the
    exact confusion `sourcestate.gate_source()` exists to catch.

    The export is in a `finally` and runs before the reclaim on every path.
    A run that failed is precisely when its run log is worth having, and
    without this the evidence dies with the copy that produced it.
    """
    if reuse is None:
        for stale in sweep(config):
            runner.note(f"reclaimed stale prefix {stale}")
    path = reuse or allocate(config, secrets.token_hex(config.prefix.name_length))
    if reuse is None:
        snapshot.populate(config.root, path, config)
    else:
        # Refreshed, not rebuilt. The source has to become what the checkout
        # says now -- that is the point of resuming after a fix -- while
        # `target/` and everything else the last run built stays put, which is
        # what makes the next attempt cheap.
        snapshot.refresh(config.root, path, config)
    status = runner.run(
        ["uv", "run", "capsem-gate", *arguments],
        cwd=path,
        env={config.environment.source_checkout: str(config.root)},
        check=False,
    )
    export(path, config.root, config)
    if status == 0:
        reclaim(config, path)
    else:
        # Kept on purpose. Its build output is what a `--prefix ... --from ...`
        # run reuses, and re-earning it costs the twenty minutes resuming
        # exists to save. `sweep` on the next run is what bounds the
        # accumulation -- not `gc`, which only reaches inside the checkout.
        runner.note(f"prefix kept for resuming: {path}")
    return status


def reclaim(config: GateConfig, path: Path) -> None:
    """Give the copy back. Each one is ~100 MB, and a fortnight of runs is a
    disk-full in the middle of the next release rather than somewhere cheap.

    This is a recursive delete of a path assembled in Python, which is the one
    shape the reclaimer guards exist to refuse, so it refuses anything that is
    not a direct child of the configured prefix root. Passing the checkout, a
    home directory or `/` reaches the `GateError` rather than `rmtree`.
    """
    root = parent_dir(config).resolve()
    resolved = Path(os.path.abspath(path))
    if resolved.parent != root or resolved == root:
        raise GateError(
            f"refusing to reclaim {resolved}: a prefix is a direct child of {root}, "
            "and this is not one"
        )
    shutil.rmtree(resolved, ignore_errors=True)
    if resolved.exists():
        # `ignore_errors` is right for a tree the gate may have chmodded, and
        # wrong as the last word: a successful run that silently kept its copy
        # is how the disk fills without anything reporting it.
        raise GateError(f"could not reclaim {resolved}; it is still on disk")

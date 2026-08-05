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
import shutil
import subprocess
from collections import defaultdict
from pathlib import Path

from . import host
from .config import GateConfig
from .errors import GateError

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


def allocate(config: GateConfig, identity: str) -> Path:
    """Reserve a named prefix, failing if something already holds the name."""
    root = parent_dir(config)
    root.mkdir(parents=True, exist_ok=True)
    path = root / identity[: config.prefix.name_length]
    if path.exists():
        raise GateError(f"prefix {path} already exists, so this run has no private tree")
    return path


def _subject(source: Path) -> list[Path]:
    """Exactly what the source digest counts.

    The same command as `scripts/source-state-digest.py`, deliberately: a
    prefix built from a different set is a prefix whose digest cannot match
    the tree it came from.
    """
    listing = subprocess.run(
        ["git", "ls-files", "-co", "--exclude-standard", "-z"],
        cwd=source,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return [Path(name) for name in listing.split("\0") if name]


def _copy_files(source: Path, target: Path, relatives: list[Path]) -> None:
    """Regular files in batches, symlinks one at a time as links.

    The split is not an optimisation. `git ls-files` lists a symlink like any
    other entry, and `cp` without `-R` follows it -- which fails outright when
    it points at a directory (`.agents/skills` in this repository, pointing at
    the one checked-in skill library) and silently duplicates a tree when it
    does not. Recreated with `os.symlink`, the copy keeps the same link and
    therefore the same digest as the tree it came from.
    """
    grouped: dict[Path, list[Path]] = defaultdict(list)
    links: list[Path] = []
    for relative in relatives:
        if (source / relative).is_symlink():
            links.append(relative)
        else:
            grouped[relative.parent].append(relative)

    flags = _CLONE if host.on_macos() else ()
    for parent, group in grouped.items():
        destination = target / parent
        destination.mkdir(parents=True, exist_ok=True)
        for index in range(0, len(group), _BATCH):
            batch = group[index : index + _BATCH]
            subprocess.run(
                ["cp", *flags, *[str(source / name) for name in batch], str(destination)],
                check=True,
            )

    for relative in links:
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.symlink_to(os.readlink(source / relative))


def _copy_carried(source: Path, target: Path, config: GateConfig) -> None:
    flags = ("-R", *_CLONE) if host.on_macos() else ("-R",)
    for relative in config.prefix.carried:
        origin = source / relative
        if not origin.exists():
            # Absent is legitimate: a fresh clone has no `private/`, and a
            # release only needs it once it signs. Refusing here would make
            # the prefix unusable for every command that never signs anything.
            continue
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["cp", *flags, str(origin), str(destination)], check=True)


def populate(source: Path, target: Path, config: GateConfig) -> None:
    """Copy the subject of the run into `target`.

    The working tree the digest counts, plus the paths `git ls-files` cannot
    see. Nothing else -- `target/` is 164 GB logical and is what makes the
    difference between a 2.2s copy and an 84s one.
    """
    target.mkdir(parents=True, exist_ok=True)
    _copy_files(source, target, _subject(source))
    _copy_carried(source, target, config)


def export(prefix: Path, destination: Path, config: GateConfig) -> None:
    """Bring back what the run produced, before the copy is reclaimed.

    Anything built inside the prefix and not named in `[prefix] exports` dies
    with it. `packages/` is the one that matters most: the signed `.pkg` a
    release publishes is built inside the run, so omitting it is a gate that
    passes with nothing to ship.
    """
    for relative in config.prefix.exports:
        origin = prefix / relative
        if not origin.exists():
            continue
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if origin.is_dir():
            shutil.copytree(origin, target, dirs_exist_ok=True)
        else:
            shutil.copy2(origin, target)


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
    if resolved.exists():
        shutil.rmtree(resolved, ignore_errors=True)

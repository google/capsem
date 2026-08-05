"""Turning a checkout into a faithful private copy of itself.

Split from `prefix`, which owns where a copy lives and when it is reclaimed.
This owns the harder half: what a copy has to contain to *be* the same subject.

Three things make that non-obvious, and each was found the expensive way.

The subject is the working tree, not `HEAD`. The source digest is
`git ls-files -co --exclude-standard`, so uncommitted edits and untracked
non-ignored files are part of what the gate qualifies -- a copy built from
`HEAD` would qualify a different tree than the one being measured.

`git ls-files` cannot see everything a run needs. `private/` is gitignored and
holds the signing keys; `.git` is not a file at all. Both are declared in
`[prefix] carried`, because the failure mode is a package lane that loses its
key during a release rather than here.

And a linked worktree cannot be copied at all -- see `_require_own_repository`.
"""

from __future__ import annotations

import os
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
#: into a running gate, which is the one thing this exists to prevent.
_CLONE = ("-c",)

#: How many paths to hand one `cp`. Per-file invocation is 2080 subprocesses;
#: one invocation is an argv the kernel refuses.
_BATCH = 200


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
        # Cleared first, because this runs a second time on every resume: `cp`
        # overwrites a regular file and `os.symlink` refuses an existing one,
        # so a reused prefix died on `FileExistsError` before any step ran.
        destination.unlink(missing_ok=True)
        destination.symlink_to(os.readlink(source / relative))


def _require_own_repository(source: Path) -> None:
    """Refuse a linked worktree, whose `.git` is a pointer rather than a repo.

    In a normal clone `.git` is a directory and copying it yields a private
    repository. In a linked worktree it is a *file* holding an absolute
    `gitdir:` path into the original checkout -- so the copy stays attached to
    the live metadata, and a commit over there changes the supposedly private
    prefix's `HEAD`. Isolation degrades back to detecting the change at
    `source.verify`, after the gate has already run, which is the failure this
    module exists to end.

    Refused rather than repaired. Making the copy self-contained means
    reproducing the common object store, and a loud refusal naming the main
    checkout is worth more than a private tree that quietly is not one.
    """
    marker = source / ".git"
    if marker.is_file():
        raise GateError(
            f"{source} is a linked worktree: its .git is a pointer into another "
            "repository, so a private copy of it would still follow that "
            "repository's HEAD. Run the gate from the main checkout."
        )


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
    _require_own_repository(source)
    target.mkdir(parents=True, exist_ok=True)
    _copy_files(source, target, _subject(source))
    _copy_carried(source, target, config)


def refresh(source: Path, target: Path, config: GateConfig) -> None:
    """Bring an existing copy's source up to date, keeping its build output.

    What `--prefix` does on every resume. Only the source is replaced; the
    `target/` the earlier run filled is the whole reason to reuse the tree.

    Copying the current subject over the old one is not enough, and the first
    version did only that. A file deleted from the source stayed in the copy,
    so a resumed run compiled and tested a tree the operator no longer had --
    with a run log and build provenance describing it as the source they
    retried. Anything the subject no longer names is removed, and the carried
    paths are refreshed too, because `.git` moves whenever the operator commits
    between attempts.
    """
    _require_own_repository(source)
    wanted = _subject(source)
    _copy_files(source, target, wanted)
    _copy_carried(source, target, config)

    keep = {target / relative for relative in wanted}
    keep |= {target / relative for relative in config.prefix.carried}
    protected = keep | {target / export for export in config.prefix.exports}
    for existing in _tracked_copies(target, config):
        if existing not in keep and not any(
            existing.is_relative_to(guard) for guard in protected
        ):
            existing.unlink(missing_ok=True)


def _tracked_copies(target: Path, config: GateConfig) -> list[Path]:
    """Files in the copy that a refresh is entitled to remove.

    Only what a previous copy put there. Build output is skipped wholesale --
    it is the reason the tree is being reused, and walking it would be walking
    tens of gigabytes to decide to keep all of it.
    """
    skip = {target / export for export in config.prefix.exports}
    skip |= {target / relative for relative in config.prefix.carried}
    found: list[Path] = []
    for path in target.rglob("*"):
        if path.is_dir() or any(path.is_relative_to(guard) for guard in skip):
            continue
        found.append(path)
    return found

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

The repository itself is cloned rather than copied -- see `_materialize_repository`.

Copying takes time, so the copy is checked rather than assumed: `digest`
measures both trees the way `source.record` does, and a copy that does not
match the checkout it came from is refused. Without that, an edit landing
during the 2.2 seconds produces a tree holding some files from before it and
some from after -- a combination that never existed at any instant in the
checkout, and which then becomes the stable subject the run records and
`source.verify` cheerfully re-asserts an hour later.
"""

from __future__ import annotations

import os
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

from . import host
from .config import GateConfig
from .errors import GateError
from .filesystem import remove
from .sourcecommit import SourceCommit

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


def digest(tree: Path, config: GateConfig) -> str:
    """Hash a tree the way `source.record` hashes the one under test.

    Through the checked-in script rather than a second implementation here.
    Two ways to hash a tree is two answers to "did it change", and the point of
    this measurement is that it is the *same* question `source.verify` asks an
    hour later -- a copy that satisfies a private definition and fails the
    shared one has proved nothing.

    Always the script belonging to the tree this process is running in, aimed
    at whichever tree is being asked about, so both answers come from one
    implementation and one environment.
    """
    completed = subprocess.run(
        [
            sys.executable,
            str(config.path(config.candidate.source_digest_script)),
            "--root",
            str(tree),
        ],
        cwd=config.root,
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or "the digest command wrote no diagnostic"
        raise GateError(f"source digest failed for {tree}: {detail}")
    return completed.stdout.strip()


def _require_faithful(source: Path, target: Path, config: GateConfig) -> None:
    """The copy has to be the checkout, not a blend of two of its moments.

    Measured after the copy in both trees rather than before and after in the
    source alone, because that is the property actually wanted: not "the source
    held still" but "this copy is that source". An edit that lands and is
    reverted during the window changes neither, and rightly so.
    """
    if digest(target, config) != digest(source, config):
        raise GateError(
            f"{source} changed while its private copy was being made, so "
            f"{target} holds files from more than one state of it. Nothing "
            "later can detect this -- the copy is frozen, so the gate would "
            "spend the whole run qualifying a tree that never existed. Run it "
            "again with the checkout left alone."
        )


def _materialize_repository(source: Path, target: Path) -> None:
    """Give the copy a repository of its own, rather than a copy of one.

    `.git` used to be carried like any other path. In a normal clone that
    works, because `.git` is a directory. In a linked worktree it is a *file*
    holding an absolute `gitdir:` path back into the original checkout, so the
    copy stayed attached to live metadata and a commit over there moved the
    supposedly private prefix's `HEAD`. That was refused outright, which made
    the gate unrunnable from a worktree -- and worktrees are how an agent gets
    an isolated tree in the first place, so the isolation machinery was
    refusing to run for exactly the people it was built for.

    Cloning answers both cases with one mechanism and no special case. On one
    filesystem `--local` hardlinks the object store; across filesystems
    `--no-local` copies it without network access. Neither uses an `alternates`
    file, so the original may be garbage-collected without pulling bytes out
    from under a running gate. The clone owns its `HEAD` and refs, so a commit
    in the source cannot move it.

    `--no-checkout` because the working tree arrives separately and writing it
    twice would be the expensive half. That leaves the index empty, so
    `read-tree` fills it from `HEAD`: `git ls-files` and `git check-ignore`
    read the index, and `faults`, `auditfs` and `sourcestate` all depend on
    them inside the prefix.

    A source with no repository at all is left alone. Receiving source as a
    tarball is a real way to get it, and `head_revision` already answers empty
    for that rather than failing.
    """
    if not (source / ".git").exists():
        return

    # Cloned beside the non-empty prefix, then moved in by same-filesystem rename.
    scratch = target.parent / f"{target.name}.gitclone"
    remove(scratch)
    remove(target / ".git")
    common = Path(
        subprocess.check_output(
            ["git", "-C", str(source), "rev-parse", "--git-common-dir"], text=True
        ).strip()
    )
    common = common if common.is_absolute() else source / common
    locality = "--local" if common.stat().st_dev == target.parent.stat().st_dev else "--no-local"
    subprocess.run(
        ["git", "clone", "--quiet", locality, "--no-checkout", str(source), str(scratch)],
        check=True,
    )
    (scratch / ".git").rename(target / ".git")
    remove(scratch)
    # Without this the index is empty, every tracked file reads as untracked,
    # and `git ls-files` names nothing.
    subprocess.run(["git", "-C", str(target), "read-tree", "HEAD"], check=True)


def _copy_carried(source: Path, target: Path, config: GateConfig) -> None:
    """Replace each carried path, rather than copying into it.

    `cp -R a b` puts `a` *inside* `b` when `b` already exists, so on every
    refresh this nested `.git` one level deeper and left the prefix's real
    repository untouched. A resumed run then measured its tree against an
    index from whenever the prefix was created: deleting a tracked file made
    `git ls-files` still name it, the digest tried to stat a path that was
    gone, and the run died in `_require_faithful` with a raw traceback before
    its first step.

    Removed first, so the copy is the source's state and not a merge of two.
    """
    flags = ("-R", *_CLONE) if host.on_macos() else ("-R",)
    for relative in config.prefix.carried:
        origin = source / relative
        destination = target / relative
        if destination.exists() or destination.is_symlink():
            remove(destination)
        if not origin.exists():
            # Absent is legitimate: a fresh clone has no `private/`, and a
            # release only needs it once it signs. Refusing here would make
            # the prefix unusable for every command that never signs anything.
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["cp", *flags, str(origin), str(destination)], check=True)


def populate_subject(source: Path, target: Path, config: GateConfig) -> None:
    """Copy only the Git-visible, source-digested subject into `target`."""
    target.mkdir(parents=True, exist_ok=True)
    _copy_files(source, target, _subject(source))
    _materialize_repository(source, target)
    _require_faithful(source, target, config)


def populate(source: Path, target: Path, config: GateConfig) -> None:
    """Copy the run subject plus separately declared ignored inputs."""
    populate_subject(source, target, config)
    _copy_carried(source, target, config)


def populate_commit(source: Path, target: Path, config: GateConfig, commit: SourceCommit) -> None:
    """Materialize a detached committed release source."""
    from . import commitsnapshot

    commitsnapshot.populate(source, target, config, commit)


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
    wanted = _subject(source)
    _copy_files(source, target, wanted)
    _copy_carried(source, target, config)
    # Re-cloned rather than updated: HEAD moves whenever the operator commits
    # between attempts, and 200ms of hardlinks is cheaper than reasoning about
    # which refs a resumed prefix is now behind on.
    _materialize_repository(source, target)

    # Only what a previous copy of the *subject* put here and the source no
    # longer names, asked with the command that defines the subject -- so an
    # ignored path is never even a candidate.
    #
    # The first version walked the whole tree and spared a hand-written list of
    # exports and carried paths. That deleted `.venv`: gitignored, therefore
    # never in the subject, therefore never "kept". The resumed run died before
    # its first step with "no Python executable was found". Everything an
    # earlier run built is in that category -- `target/`, `node_modules`, the
    # venv -- and deleting it deletes the entire reason to reuse the tree.
    for relative in set(_subject(target)) - set(wanted):
        (target / relative).unlink(missing_ok=True)

    # The same check as a fresh copy, and a sharper one here: a refresh has to
    # remove what the source no longer names, and a deletion pass that stops
    # working leaves a resumed run compiling a tree the operator does not have.
    _require_faithful(source, target, config)

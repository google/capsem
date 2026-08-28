"""Filesystem primitives, and the two that carry a safety property.

Most of these wrap one call so it can be rendered in a dry run and timed in a
run log. Two are not wrappers, and both encode a defect this project has met.

`AtomicReplace` exists because the initrd is a hash-named file hardlinked into
every asset tree built from the same bytes. Rewriting it in place rewrites all
of them, and the damage surfaces later as a VM that will not boot from a tree
nobody touched. `_pack-initrd` wrote `${INITRD}.tmp.$$` and moved it for
exactly this reason -- a rule that lived in one recipe, in shell, with nothing
enforcing it.

`Symlink` exists because `assets/current` is repointed by whichever image
builder finished last. The host-architecture VM proof that follows has to aim
it deliberately and then check where it landed, because a proof that ran
against the other architecture's assets passes just as readily.

Every path here arrives as a `Path` the caller resolved from configuration.
Nothing in this module knows a filename.
"""

from __future__ import annotations

import os
import shutil
from collections.abc import Callable
from pathlib import Path

from .actions import Action
from .context import Context
from .errors import GateError
from .filesystem import (
    DIGESTS,
    copy_file,
    copy_tree,
    digest_of,
    discard,
    link,
    make_dir,
    merge_tree,
    remove,
    scratch_dir,
    write_text,
)

# Re-exported: the actions below are these operations wrapped for a plan, and
# splitting the file should not move every call site.
__all__ = [
    "DIGESTS",
    "copy_file",
    "copy_tree",
    "digest_of",
    "discard",
    "link",
    "make_dir",
    "merge_tree",
    "remove",
    "scratch_dir",
    "write_text",
]


class MakeDir(Action, name="make-dir"):
    """Create a directory and its parents, tolerating one already there."""

    def __init__(self, path: Path) -> None:
        self._path = path

    def render(self) -> str:
        return f"mkdir -p {self._path}"

    def perform(self, context: Context) -> None:
        if context.observing:
            return
        self._path.mkdir(parents=True, exist_ok=True)


class Remove(Action, name="remove"):
    """Delete a file or a whole tree, tolerating its absence and nothing else.

    Teardown runs against whatever state the failure left behind, which may be
    nothing at all -- so "it was not there" is the expected case, not an error.

    Absence is the *only* tolerable outcome. `ignore_errors=True` made every
    removal succeed on paper, so a busy or unwritable path survived into the
    next qualification while the plan recorded the cleanup as done, and
    retention reported bytes it had reclaimed that were still on the disk. A
    cleanup that cannot happen has to say so.
    """

    def __init__(self, path: Path) -> None:
        self._path = path

    def render(self) -> str:
        return f"rm -rf {self._path}"

    def perform(self, context: Context) -> None:
        if context.observing:
            return
        remove(self._path)


class Copy(Action, name="copy"):
    """Copy a file or a tree, merging into an existing destination tree."""

    def __init__(self, source: Path, target: Path) -> None:
        self._source = source
        self._target = target

    def render(self) -> str:
        return f"cp -r {self._source} {self._target}"

    def perform(self, context: Context) -> None:
        if context.observing:
            return
        if not self._source.exists():
            raise GateError(f"nothing to copy: {self._source} does not exist")
        self._target.parent.mkdir(parents=True, exist_ok=True)
        if self._source.is_dir():
            # The shared merge, which never follows a symlink on either side.
            # `shutil.copytree(dirs_exist_ok=True)` writes *through* a
            # destination symlink into whatever it points at.
            merge_tree(self._source, self._target)
        else:
            shutil.copy2(self._source, self._target)


class Symlink(Action, name="symlink"):
    """Point a link at a target, replacing any link already there.

    Refuses to replace a real directory. Removing a populated tree because a
    link was expected in its place is not a mistake anything recovers from,
    and it is a plausible one: `assets/current` is a link, and the thing
    beside it with the same shape is an architecture's whole asset tree.
    """

    def __init__(self, link: Path, target: str) -> None:
        self._link = link
        self._target = target

    def render(self) -> str:
        return f"ln -sfn {self._target} {self._link}"

    def perform(self, context: Context) -> None:
        if context.observing:
            return
        if self._link.exists() and not self._link.is_symlink():
            raise GateError(f"{self._link} is not a symlink; refusing to replace it")
        self._link.unlink(missing_ok=True)
        self._link.parent.mkdir(parents=True, exist_ok=True)
        self._link.symlink_to(self._target)

        landed = self._link.readlink()
        if landed.name != Path(self._target).name:
            raise GateError(f"{self._link} points at {landed}, not {self._target}")


class AtomicReplace(Action, name="atomic-replace"):
    """Build new contents beside a file, then move them into its name.

    `os.replace` puts a new inode under the name and leaves the old one alone,
    so anything else hardlinked to the previous contents keeps them. Writing
    the target directly would edit every one of those trees at once.
    """

    def __init__(self, target: Path, build: Callable[[Path], None]) -> None:
        self._target = target
        self._build = build

    def render(self) -> str:
        return f"build {self._target}.tmp and mv it onto {self._target}"

    def perform(self, context: Context) -> None:
        if context.observing:
            return
        scratch = self._target.with_name(f"{self._target.name}.tmp.{os.getpid()}")
        self._target.parent.mkdir(parents=True, exist_ok=True)
        try:
            self._build(scratch)
        except BaseException:
            # A half-written artifact that looks whole is worse than none: the
            # next step reads it, and the failure surfaces somewhere else.
            scratch.unlink(missing_ok=True)
            raise
        os.replace(scratch, self._target)


class Hash(Action, name="hash"):
    """Record what an artifact's bytes are, in the run log.

    So a run answers "which bytes did this gate build" afterwards, without
    re-hashing a tree that rotation or `gc` may already have reclaimed.
    """

    def __init__(self, path: Path) -> None:
        self._path = path

    def render(self) -> str:
        return f"hash {self._path}"

    def perform(self, context: Context) -> None:
        if context.observing:
            # Nothing built it, because nothing ran.
            return
        if not self._path.is_file():
            raise GateError(f"cannot hash {self._path}: it is not a file")
        algorithm = context.config.runlog.artifact_digest
        context.journal.artifact(
            self._path,
            digest=digest_of(self._path, algorithm=algorithm),
            size=self._path.stat().st_size,
        )


class RequireFile(Action, name="require-file"):
    """Fail here, naming the file, rather than three steps later."""

    def __init__(self, path: Path) -> None:
        self._path = path

    def render(self) -> str:
        return f"test -f {self._path}"

    def perform(self, context: Context) -> None:
        if not self._path.is_file():
            raise GateError(f"required file is missing: {self._path}")


class RequireNonEmpty(Action, name="require-non-empty"):
    """Present is not the same as built.

    A build that fails after creating its output leaves a zero-length file that
    passes every existence check, which is how an empty rootfs reached a boot.
    """

    def __init__(self, path: Path) -> None:
        self._path = path

    def render(self) -> str:
        return f"test -s {self._path}"

    def perform(self, context: Context) -> None:
        if not self._path.is_file():
            raise GateError(f"required file is missing: {self._path}")
        if self._path.stat().st_size == 0:
            raise GateError(f"required file is empty: {self._path}")

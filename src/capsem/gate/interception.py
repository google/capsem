"""Proxy the primitives that change the disk, so nothing can avoid being seen.

An external watcher answers "something happened here, shortly before I
looked". That leaves three gaps this closes: the state at the moment of the
call is already gone by the time a notification arrives, the caller is
unknown, and anything the platform coalesces is simply missing.

Wrapping the call has none of those. `os.chmod` is observed with the mode
before *and* after; `os.link` is observed knowing both paths; and a primitive
added tomorrow that nobody remembers to instrument is caught by
`test_every_mutating_primitive_the_stdlib_offers_is_intercepted`, which reads
this list against the standard library rather than trusting it.

In-process only, by construction: a subprocess makes its own syscalls, and
those stay the external watcher's job.
"""

from __future__ import annotations

import contextvars
import functools
import os
import shutil
import stat
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Protocol

#: The step whose thread is executing. A `ContextVar` rather than "everything
#: in flight", because the scheduler runs steps in worker threads and the
#: caller is then known exactly instead of narrowed to a set.
CURRENT_STEP: contextvars.ContextVar[str | None] = contextvars.ContextVar(
    "capsem_gate_step", default=None
)


class Observer(Protocol):
    """What interception needs from whatever is judging."""

    def observed(self, kind: str, path: Path, *, before: int | None = None) -> None: ...


class Instrument:
    """Patch the mutating primitives for the duration of a run."""

    #: (module, attribute, kind). Data, so a test can compare it against what
    #: the standard library actually offers instead of a reviewer noticing.
    TARGETS: tuple[tuple[object, str, str], ...] = (
        (os, "link", "link"),
        (os, "symlink", "symlink"),
        (os, "unlink", "unlink"),
        (os, "remove", "unlink"),
        (os, "rmdir", "unlink"),
        (os, "chmod", "chmod"),
        (os, "rename", "rename"),
        (os, "replace", "rename"),
        (os, "truncate", "write"),
        (shutil, "copy", "copy"),
        (shutil, "copy2", "copy"),
        (shutil, "copyfile", "copy"),
        (shutil, "copytree", "copy"),
        (shutil, "rmtree", "unlink"),
        (shutil, "move", "rename"),
    )

    #: Calls whose *destination* is the interesting path: `link(src, dst)`
    #: creates `dst`, and `dst` is what now shares an inode it should not.
    #:
    #: `symlink` belongs here and was missing. `os.symlink(src, dst)` creates
    #: `dst` like the rest, so recording the first argument recorded the link's
    #: *target* -- and `Path.symlink_to("arm64")` passes a bare relative name,
    #: which `resolve()` then anchored to the checkout root. The result was a
    #: report that `<root>/arm64` had been written: a path no step touched, not
    #: gitignored, and therefore classified as source. Harmless as a log line
    #: for as long as faults were only logged; the moment a source-tree fault
    #: began aborting releases it stopped a release at `assets.assemble`.
    DESTINATION_IS_SECOND = frozenset({"link", "copy", "rename", "symlink"})

    def __init__(self, observer: Observer, *, fd_path_template: str) -> None:
        self._observer = observer
        self._fd_path_template = fd_path_template
        self._saved: list[tuple[object, str, object]] = []

    def __enter__(self) -> Instrument:
        for module, name, kind in self.TARGETS:
            original = getattr(module, name)
            self._saved.append((module, name, original))
            setattr(module, name, self._wrap(original, kind))
        return self

    def __exit__(self, *_: object) -> None:
        for module, name, original in reversed(self._saved):
            setattr(module, name, original)
        self._saved.clear()

    def _wrap(self, original: Callable[..., object], kind: str) -> Callable[..., object]:
        observer = self._observer
        template = self._fd_path_template

        @functools.wraps(original)
        def proxy(*args: object, **kwargs: object) -> object:
            subject = _subject(kind, args)
            before = _mode_of(subject) if kind == "chmod" else None
            result = original(*args, **kwargs)
            located = _locate(subject, kwargs, template)
            if located is not None:
                observer.observed(kind, located, before=before)
            return result

        return proxy


def _subject(kind: str, args: tuple[object, ...]) -> object | None:
    if kind in Instrument.DESTINATION_IS_SECOND and len(args) > 1:
        return args[1]
    return args[0] if args else None


#: macOS `fcntl` command for "give me this descriptor's path". A number in the
#: platform's ABI, not a path, so it stays here; Linux answers the same
#: question by reading a symlink whose template comes from config.
_F_GETPATH = 50

#: Enough for `PATH_MAX` on both platforms.
_PATH_BUFFER = 1024


def _path_of_fd(handle: int, template: str) -> Path | None:
    """The path behind an open descriptor, or `None` if it cannot be had."""
    try:
        if sys.platform == "darwin":
            import fcntl

            answer = fcntl.fcntl(handle, _F_GETPATH, bytes(_PATH_BUFFER))
            return Path(os.fsdecode(answer.rstrip(b"\0")))
        return Path(os.readlink(template.format(handle=handle)))
    except (OSError, ValueError, UnicodeDecodeError):
        return None


def _locate(subject: object, kwargs: dict[str, object], template: str) -> Path | None:
    """Where the call actually acted, not what the caller happened to spell.

    `shutil.rmtree` deletes through a directory descriptor -- `os.unlink(
    'profile.toml', dir_fd=5)` -- and recording the bare entry name left the
    path to be resolved against the current working directory, which for the
    gate is the checkout root. One release run logged 42 faults that way, each
    naming a tracked file nothing had touched.

    Returns `None` when the path cannot be established, and the judge then
    declines to call it a source mutation: a fault nobody can locate is not
    evidence, and inventing one is worse than missing it.
    """
    if isinstance(subject, int):
        # An integer subject is a descriptor, as in `os.truncate(fd, n)`.
        return _path_of_fd(subject, template)
    if not isinstance(subject, str | os.PathLike):
        return None
    # `str`, not `os.fsdecode`, for the same reason `_mode_of` does it: the
    # union leaves `PathLike[object]`, which the checker will not accept.
    path = Path(str(subject))
    if path.is_absolute():
        return path
    handle = kwargs.get("dir_fd")
    if handle is None:
        return Path.cwd() / path
    if not isinstance(handle, int):
        return None
    anchor = _path_of_fd(handle, template)
    return anchor / path if anchor is not None else None


def _mode_of(path: object) -> int | None:
    """The mode as it is *right now*, which after the call is unrecoverable."""
    # `str` only. Every proxied caller passes a path, and accepting the whole
    # `PathLike` union leaves `PathLike[object]` for the checker to reject --
    # a file descriptor has no mode to read here anyway.
    if not isinstance(path, str | os.PathLike):
        return None
    try:
        return stat.S_IMODE(os.stat(str(path)).st_mode)
    except (OSError, ValueError):
        return None

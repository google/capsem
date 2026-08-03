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
    DESTINATION_IS_SECOND = frozenset({"link", "copy", "rename"})

    def __init__(self, observer: Observer) -> None:
        self._observer = observer
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

        @functools.wraps(original)
        def proxy(*args: object, **kwargs: object) -> object:
            subject = _subject(kind, args)
            before = _mode_of(subject) if kind == "chmod" else None
            result = original(*args, **kwargs)
            if subject is not None:
                observer.observed(kind, Path(str(subject)), before=before)
            return result

        return proxy


def _subject(kind: str, args: tuple[object, ...]) -> object | None:
    if kind in Instrument.DESTINATION_IS_SECOND and len(args) > 1:
        return args[1]
    return args[0] if args else None


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

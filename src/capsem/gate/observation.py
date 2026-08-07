"""What the run did to the filesystem, judged as it happens.

`contends` is a list an author typed and `can_overlap` compares two such lists
to each other. Neither has ever looked at a disk, so the invariant actually
enforced is "the declarations agree with each other" -- and a step that does
not mention what it touches satisfies every check by saying nothing. The
writer is frequently not the step at all, but a unit test three subprocesses
below it.

Collecting and judging are one pass on purpose. A separate analyzer is a thing
that runs later, which in practice means a thing that runs never: a rule
evaluated after a sixty-minute gate is a rule nobody acts on until someone
thinks to read a file.

Two sources feed it. `interception.Instrument` wraps the in-process primitives
and is exact -- the caller, and the state before the call. A `watchdog`
observer covers what subprocesses do, which no in-process proxy can see, and
is best-effort: it is notified and then stats, so a change reverted inside
that window is already gone when it looks.
"""

from __future__ import annotations

import stat
import threading
import time
from collections.abc import Callable, Iterable, Mapping
from pathlib import Path
from types import TracebackType
from typing import TYPE_CHECKING

from .faults import Event, Fault, facts_of, ignored_here, source_inodes
from .interception import CURRENT_STEP

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from watchdog.observers.api import BaseObserver


class Watch:
    """Observes the paths a run must not disturb and judges each change live."""

    def __init__(
        self,
        roots: Iterable[Path],
        *,
        source_root: Path,
        declared: Mapping[str, frozenset[str]] | None = None,
        on_fault: Callable[[Fault], None] | None = None,
    ) -> None:
        self._roots = [root for root in roots if root.exists()]
        self._source_root = source_root.resolve()
        self._source_inodes = source_inodes(self._source_root)
        self._declared = dict(declared or {})
        self._on_fault = on_fault
        self.faults: list[Fault] = []
        self.events: list[Event] = []

        self._live: set[str] = set()
        self._lock = threading.Lock()
        self._observer: BaseObserver | None = None
        self._modes: dict[Path, list[int]] = {}
        self._digests: dict[str, Path] = {}
        self._reported: set[tuple[Path, str]] = set()

    # -- step attribution ---------------------------------------------------

    def entered(self, label: str) -> None:
        with self._lock:
            self._live.add(label)

    def left(self, label: str) -> None:
        with self._lock:
            self._live.discard(label)

    # -- lifecycle ----------------------------------------------------------

    def __enter__(self) -> Watch:
        from watchdog.events import FileSystemEventHandler
        from watchdog.observers import Observer

        watch = self

        class _Handler(FileSystemEventHandler):
            def on_any_event(self, event) -> None:
                if event.is_directory or event.event_type == "opened":
                    return
                watch.observed(event.event_type, Path(str(event.src_path)))

        observer = Observer()
        for root in self._roots:
            observer.schedule(_Handler(), str(root), recursive=True)
        observer.start()
        self._observer = observer
        self.survey()
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        observer = self._observer
        if observer is None:
            return
        observer.stop()
        # Bounded: a hung observer thread must not outlive the run it watched.
        observer.join(timeout=5)
        self._observer = None

    # -- observation --------------------------------------------------------

    def observed(self, kind: str, path: Path, *, before: int | None = None) -> None:
        """One change, from either source.

        `before` is the mode a moment ago, which only an intercepted caller
        can supply: after the call it is simply the current mode, and the
        transition is what makes a concurrent reader fail.
        """
        step = CURRENT_STEP.get()
        if step is not None:
            steps: tuple[str, ...] = (step,)
        else:
            with self._lock:
                steps = tuple(sorted(self._live))
        event = Event(at=time.time(), kind=kind, path=path, steps=steps, facts=facts_of(path))
        self.events.append(event)
        if before is not None:
            history = self._modes.setdefault(path, [])
            if not history:
                history.append(before)
        self._judge(event)

    def _fault(self, event: Event, reason: str, detail: str) -> None:
        key = (event.path, reason)
        if key in self._reported:
            return
        self._reported.add(key)
        fault = Fault(path=event.path, steps=event.steps, reason=reason, detail=detail)
        self.faults.append(fault)
        if self._on_fault is not None:
            self._on_fault(fault)

    def _judge(self, event: Event) -> None:
        source = self.is_source(event.path)

        if source and event.kind != "deleted":
            self._fault(
                event, "source-tree", f"{event.kind} during the run; the gate qualifies this tree"
            )

        if not source and event.inode is not None and event.links and event.links > 1:
            origin = self._source_inodes.get(event.inode)
            if origin is not None:
                self._fault(
                    event,
                    "hardlinked-source",
                    f"shares inode {event.inode} (nlink={event.links}) with checked-in "
                    f"{origin.relative_to(self._source_root)}, so a chmod here rewrites "
                    "tracked source and no content digest will notice",
                )

        if event.mode is not None:
            history = self._modes.setdefault(event.path, [])
            # Returning to any mode already seen -- not merely the one before
            # last -- is the flip-flop: 0644 -> 0000 -> 0644 is identical at
            # both ends and unreadable in the middle.
            if history and event.mode != history[-1] and event.mode in history[:-1]:
                self._fault(
                    event,
                    "mode-flip-flop",
                    f"{history[-1]:04o} -> {event.mode:04o}, back to a mode it already had; "
                    "a concurrent reader sees the middle state and fails intermittently",
                )
            if not history or history[-1] != event.mode:
                history.append(event.mode)
            if source and event.mode & (stat.S_IWGRP | stat.S_IWOTH):
                self._fault(
                    event, "over-permission", f"mode {event.mode:04o} is writable beyond its owner"
                )

        if event.digest is not None:
            first = self._digests.setdefault(event.digest, event.path)
            if first != event.path and event.inode is not None:
                self._fault(event, "duplicate-content", f"identical bytes already at {first}")

        if len(event.steps) >= 2 and not source:
            shared = set.intersection(
                *(set(self._declared.get(step, frozenset())) for step in event.steps)
            )
            if not shared:
                self._fault(
                    event,
                    "undeclared-contention",
                    "two steps the scheduler ran together both touched this, and neither "
                    "declares sharing it",
                )

    def is_source(self, path: Path) -> bool:
        from .faults import BUILD_OUTPUT

        # Absolute only. `Path.resolve()` anchors a relative path to the
        # current working directory, which for the gate is the checkout root --
        # so a bare `profile.toml` from a `dir_fd` caller resolved into the
        # source tree and was reported as a mutation of a file the run never
        # touched. Refusing to judge what was not located keeps that class out
        # regardless of which caller spells a path loosely; `interception`
        # resolves the ones it can.
        if not path.is_absolute():
            return False
        try:
            relative = path.resolve().relative_to(self._source_root)
        except (ValueError, OSError):
            return False
        if not relative.parts or relative.parts[0] in BUILD_OUTPUT:
            return False
        # And then git, which knows what a hand-written set cannot. The names
        # above stay: a fixture is not always a repository, and git answers
        # nothing outside one -- which would make every path "source" and turn
        # nine of this file's own tests red. So the two are a union, not a
        # replacement.
        #
        # Without this, nothing nested could ever be recognised, because only
        # the first component was compared. `crates/capsem-app/gen/` is
        # gitignored Tauri output and was reported as a source-tree fault on
        # every run; widening the set is whack-a-mole, since the next generated
        # directory lands somewhere else again.
        return not ignored_here(self._source_root, relative.parent)

    # -- state, not moments -------------------------------------------------

    def survey(self) -> None:
        """Report what is already wrong before this run touched anything.

        A hardlink made by a previous run produces no event in this one, and
        the defect is in the state: the shared inode is there whether or not
        anybody relinks it today. Without this the check fires only on the run
        that happens to rebuild, which is a coincidence rather than a guard.
        """
        for root in self._roots:
            if self.is_source(root):
                continue
            for path in root.rglob("*"):
                try:
                    info = path.stat()
                except OSError:
                    continue
                if not stat.S_ISREG(info.st_mode) or info.st_nlink < 2:
                    continue
                origin = self._source_inodes.get(info.st_ino)
                if origin is None:
                    continue
                self.observed("pre-existing", path)

    def sweep(self) -> list[Fault]:
        """What only the end of a run can decide.

        An artifact that is empty once everything has finished is a failed
        build that reported success; during the run it is a file being written.
        """
        for path in {event.path for event in self.events if not self.is_source(event.path)}:
            try:
                if path.is_file() and path.stat().st_size == 0:
                    self._fault(
                        Event(at=time.time(), kind="final", path=path, steps=()),
                        "empty-artifact",
                        "zero bytes at the end of the run",
                    )
            except OSError:
                continue
        return self.faults

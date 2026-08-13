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

`interception.Instrument` is exact about in-process callers and prior state. A
`watchdog` observer covers what subprocesses do, which no proxy can see, and
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

from . import faultrules
from .errors import GateError
from .faults import (
    Attribution,
    Event,
    Fault,
    facts_of,
    is_source,
    source_inodes,
)
from .interception import CURRENT_STEP

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from watchdog.observers.api import BaseObserver


#: The one fault reason a release refuses rather than records. Named here
#: because `observing` compares against it: a literal in both places is two
#: spellings of one rule, and the release would stop refusing the day either
#: drifted.

# Linux inotify/watchdog emits this when a descriptor opened only for reading
# is closed.  It is explicitly evidence that no write happened, not a
# best-effort change notification.  Keep it out before `facts_of` hashes the
# path: a build reads thousands of files and otherwise makes unbounded false mutations.
READ_ONLY_CLOSE = "closed_no_write"


class Watch:
    """Observes the paths a run must not disturb and judges each change live."""

    def __init__(
        self,
        roots: Iterable[Path],
        *,
        source_root: Path,
        declared: Mapping[str, frozenset[str]] | None = None,
        on_fault: Callable[[Fault], None] | None = None,
        duplicate_content_exempt: Iterable[str] = (),
    ) -> None:
        self._roots = [root for root in roots if root.exists()]
        self._source_root = source_root.resolve()
        self._source_inodes = source_inodes(self._source_root)
        self._declared = dict(declared or {})
        self._on_fault = on_fault
        self._duplicate_exempt = tuple(duplicate_content_exempt)
        self.faults: list[Fault] = []
        self.events: list[Event] = []

        self._live: set[str] = set()
        self._lock = threading.Lock()
        self._refusal_ready = threading.Condition(self._lock)
        self._refusals_recording = 0
        self._observer: BaseObserver | None = None
        self._modes: dict[Path, list[int]] = {}
        self._digests: dict[str, Path] = {}
        self._reported: set[tuple[Path, str]] = set()
        self._refusal: str | None = None

    # -- step attribution ---------------------------------------------------

    def entered(self, label: str) -> None:
        with self._lock:
            self._live.add(label)

    def left(self, label: str) -> None:
        with self._lock:
            self._live.discard(label)

    def refuse(self, message: str) -> None:
        """Hand an asynchronous refusal to the thread driving the plan."""
        with self._lock:
            self._refusal = self._refusal or message

    def refuse_after(self, message: str, evidence: Callable[[], None]) -> None:
        """Keep checkpoints behind the durable evidence that explains a refusal."""
        with self._lock:
            self._refusals_recording += 1
        try:
            evidence()
        finally:
            with self._refusal_ready:
                self._refusal = self._refusal or message
                self._refusals_recording -= 1
                self._refusal_ready.notify_all()

    def checkpoint(self) -> None:
        """Raise a fresh exception on the controlling thread at a safe boundary."""
        with self._refusal_ready:
            while self._refusals_recording:
                self._refusal_ready.wait()
            refusal = self._refusal
        if refusal is not None:
            raise GateError(refusal)

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
        if kind == READ_ONLY_CLOSE:
            return
        step = CURRENT_STEP.get()
        if step is not None:
            steps: tuple[str, ...] = (step,)
            attribution: Attribution = "exact"
        else:
            with self._lock:
                steps = tuple(sorted(self._live))
            # Watchdog dispatches on its own thread, so the ContextVar cannot
            # name the writer. One live step is the only possible gate writer;
            # two or more are diagnostic candidates, not a claim that every
            # one touched the path.
            attribution = "exact" if len(steps) <= 1 else "candidates"
        event = Event(
            at=time.time(),
            kind=kind,
            path=path,
            steps=steps,
            facts=facts_of(path),
            attribution=attribution,
        )
        self.events.append(event)
        if before is not None:
            history = self._modes.setdefault(path, [])
            if not history:
                history.append(before)
        self._judge(event)

    def fault(self, event: Event, reason: str, detail: str) -> None:
        key = (event.path, reason)
        if key in self._reported:
            return
        self._reported.add(key)
        fault = Fault(path=event.path, steps=event.steps, reason=reason, detail=detail)
        self.faults.append(fault)
        if self._on_fault is not None:
            self._on_fault(fault)

    def _judge(self, event: Event) -> None:
        """Apply every rule to one event; the rules live in `faultrules`."""
        faultrules.judge(self, event)

    def is_source(self, path: Path) -> bool:
        return is_source(path, self._source_root)

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
                    self.fault(
                        Event(at=time.time(), kind="final", path=path, steps=()),
                        "empty-artifact",
                        "zero bytes at the end of the run",
                    )
            except OSError:
                continue
        return self.faults

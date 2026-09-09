"""Close watchdog's FSEvents thread-start/native-registration gap.

Watchdog 6 starts the stream in ``run``, after ``start`` has returned. A
write in that interval can be lost even when history is enabled. Keep its
event decoding and Unicode path handling, but acknowledge native registration
before returning control to the work being observed.
"""

from __future__ import annotations

import threading
import time
from collections.abc import Callable
from pathlib import Path

from watchdog.events import FileSystemEventHandler
from watchdog.observers.api import BaseObserver

from .errors import GateError
from .host import on_macos


def file_handler(notify: Callable[[str, Path], None]) -> FileSystemEventHandler:
    class Handler(FileSystemEventHandler):
        def on_any_event(self, event) -> None:
            if event.is_directory:
                # Directory-only notifications still carry changes to children.
                if event.event_type in {"created", "moved", "modified"}:
                    directory = Path(str(event.dest_path or event.src_path))
                    children = (directory.glob("*") if event.event_type == "modified"
                                else directory.rglob("*"))
                    for child in children:
                        if child.is_file():
                            notify("created", child)
            elif event.event_type != "opened":
                notify(event.event_type, Path(str(event.src_path)))

    return Handler()


def ready_observer(observer: BaseObserver, *, timeout: float) -> BaseObserver:
    if not on_macos():
        return observer

    from watchdog.observers.fsevents import FSEventsEmitter, FSEventsObserver, _fsevents

    if not isinstance(observer, FSEventsObserver):
        return observer

    class ReadyEmitter(FSEventsEmitter):
        def join(self, timeout: float | None = None) -> None:
            # Watchdog's failed-start cleanup otherwise joins without a bound.
            super().join(timeout=timeout if timeout is not None else registration_timeout)

        def start(self) -> None:
            self.registered = threading.Event()
            self.registration_error: Exception | None = None
            super().start()
            if not self.registered.wait(registration_timeout):
                self.stop()
                raise GateError(f"filesystem observer registration timed out: {self.watch.path}")
            if self.registration_error is not None:
                self.stop()
                raise GateError(
                    f"filesystem observer registration failed: {self.watch.path}: "
                    f"{self.registration_error}"
                ) from self.registration_error

        def run(self) -> None:
            self.pathnames = [self.watch.path]
            self._start_time = time.monotonic()
            try:
                _fsevents.add_watch(self, self.watch, self.events_callback, self.pathnames)
            except Exception as error:
                self.registration_error = error
                self.registered.set()
                return
            # A timed-out start may have stopped us before add_watch returned.
            if not self.should_keep_running():
                _fsevents.remove_watch(self.watch)
                return
            self.registered.set()
            _fsevents.read_events(self)

    registration_timeout = timeout

    class ReadyObserver(FSEventsObserver):
        def __init__(self) -> None:
            BaseObserver.__init__(self, ReadyEmitter)

    return ReadyObserver()

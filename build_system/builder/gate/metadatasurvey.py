"""A metadata backstop for native filesystem notifications dropped under load."""

from __future__ import annotations

import os
import threading
from collections.abc import Callable, Iterable
from pathlib import Path

from .faults import file_stamp


def _scan(roots: Iterable[Path]) -> dict[Path, tuple[int, ...]]:
    states = {}
    for root in roots:
        for directory, _, names in os.walk(root):
            for name in names:
                path = Path(directory) / name
                if (stamp := file_stamp(path)) is not None:
                    states[path] = stamp
    return states


class MetadataSurvey(threading.Thread):
    def __init__(
        self, roots: Iterable[Path], notify: Callable[[str, Path], None],
        refuse: Callable[[str], None], *, interval: float,
    ) -> None:
        super().__init__(daemon=True)
        self._roots = tuple(roots)
        self._states = _scan(self._roots)
        self._notify = notify
        self._refuse = refuse
        self._interval = interval
        self._stopped = threading.Event()

    def run(self) -> None:
        try:
            while not self._stopped.wait(self._interval):
                current = _scan(self._roots)
                for path, stamp in current.items():
                    if stamp != self._states.get(path):
                        self._notify("modified" if path in self._states else "created", path)
                for path in self._states.keys() - current.keys():
                    self._notify("deleted", path)
                self._states = current
        except Exception as error:
            self._refuse(f"filesystem metadata observation failed: {error}")

    def stop(self) -> None:
        self._stopped.set()

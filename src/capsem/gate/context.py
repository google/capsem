"""What an action is handed, instead of what it happened to close over.

Before this, a unit of gate work was a closure over whatever its module had in
scope: one module reached for `self._config`, another rebuilt the config from
`runner.root`, a third took the path it needed as a constructor argument. Three
routes to one value, and no way to move a piece of work between commands
without dragging its module along.

An action receives a `Context` and reaches for nothing else. That is what makes
the same step reusable in a command that sequences it differently, and it is
what lets a test hand an action a recording runner and a list.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Protocol

from .config import GateConfig
from .proc import Runner


class Journal(Protocol):
    """The part of the run log an action is allowed to write to.

    A protocol rather than the concrete run log, so the primitives do not
    depend on how a run is stored and a test can pass a list. Deliberately
    narrow: an action reports what it produced, it does not decide what a step
    or a run means.
    """

    def note(self, message: str) -> None:
        """Record something worth reading back, without failing anything."""

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        """Record bytes this run produced.

        So a run log can answer "which bytes did this gate build" without
        re-hashing an asset tree that may already have been reclaimed.
        """


class NullJournal:
    """Writes nothing.

    The default, so an action can be exercised -- in a test, or in a command
    that has not opened a run log yet -- without one.
    """

    def note(self, message: str) -> None:
        """Discarded."""

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        """Discarded."""


@dataclass(frozen=True)
class Context:
    """Everything an action needs, and nothing it does not."""

    runner: Runner
    config: GateConfig
    journal: Journal = field(default_factory=NullJournal)

    env: Mapping[str, str] = field(default_factory=dict)
    """Environment every action in this scope adds to its own.

    A workspace exports `CAPSEM_HOME` once, here, rather than every command
    inside it remembering to pass it -- which is how one of them stops
    remembering.
    """

    @property
    def root(self) -> Path:
        """The checkout the gate is running against."""
        return self.config.root

    def path(self, relative: str) -> Path:
        return self.config.path(relative)

    def with_env(self, **extra: str) -> Context:
        """A child context adding environment, leaving this one untouched.

        Frozen and copied rather than mutated, so a step that adds an
        environment variable cannot change what a concurrent step sees.
        """
        return replace(self, env={**self.env, **extra})

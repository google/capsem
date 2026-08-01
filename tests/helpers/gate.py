"""A `Runner` that records commands instead of running them.

The defects this refactor exists to prevent are ordering defects: a manifest
URL consumed before anything wrote the manifest, a container reused after it
was removed, storage released before the rail that needed it finished. None of
those is visible in a single command, so a test that stubs `subprocess` call by
call cannot see them either.

`RecordingRunner` keeps the whole sequence, and `index_of` turns "A must
precede B" into an assertion.
"""

from __future__ import annotations

import re
import subprocess
from collections.abc import Iterable, Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import TextIO

from capsem.gate.proc import Command, Runner


class RecordingRunner(Runner):
    """Records every command; answers with canned output.

    `replies` maps a substring of the rendered command to the stdout that
    command should produce. `failures` does the same for exit statuses, so a
    test can make one step fail and assert what the gate does next.
    """

    def __init__(
        self,
        root: Path,
        *,
        replies: dict[str, str] | None = None,
        failures: Iterable[str] = (),
        stream: TextIO | None = None,
    ) -> None:
        super().__init__(root, stream=stream)
        self.commands: list[Command] = []
        self.notes: list[str] = []
        self._replies = dict(replies or {})
        self._failures = tuple(failures)

    # -- Runner overrides --------------------------------------------------

    def execute(self, command: Command) -> subprocess.CompletedProcess[str]:
        self.commands.append(command)
        rendered = str(command)
        status = 1 if any(marker in rendered for marker in self._failures) else 0
        stdout = ""
        for marker, reply in self._replies.items():
            if marker in rendered:
                stdout = reply
                break
        return subprocess.CompletedProcess(
            args=list(command.argv), returncode=status, stdout=stdout, stderr=""
        )

    def step(self, message: str) -> None:
        self.notes.append(message)

    def note(self, message: str) -> None:
        self.notes.append(message)

    def fail_on(self, *markers: str) -> None:
        """Change what fails partway through, for before/after checks."""
        self._failures = markers

    # -- assertions --------------------------------------------------------

    @property
    def rendered(self) -> list[str]:
        return [str(command) for command in self.commands]

    def index_of(self, pattern: str) -> int:
        """Position of the first command matching `pattern` as a regex.

        Fails loudly rather than returning -1: a missing command and a
        mis-ordered one are different bugs, and `assert a < b` on a -1 quietly
        reports the wrong one.
        """
        expression = re.compile(pattern)
        for position, rendered in enumerate(self.rendered):
            if expression.search(rendered):
                return position
        raise AssertionError(
            f"no command matched {pattern!r}; ran:\n  " + "\n  ".join(self.rendered)
        )

    def last_index_of(self, pattern: str) -> int:
        """Position of the *last* match.

        Some commands legitimately run twice -- `docker rm -f` both clears a
        predecessor and tears this run down -- and asserting on the first
        occurrence would prove the wrong one happened.
        """
        expression = re.compile(pattern)
        for position in reversed(range(len(self.rendered))):
            if expression.search(self.rendered[position]):
                return position
        raise AssertionError(
            f"no command matched {pattern!r}; ran:\n  " + "\n  ".join(self.rendered)
        )

    def matching(self, pattern: str) -> list[str]:
        expression = re.compile(pattern)
        return [line for line in self.rendered if expression.search(line)]

    def ran(self, pattern: str) -> bool:
        return bool(self.matching(pattern))

    def assert_order(self, *patterns: str) -> None:
        """Assert the given commands ran, in the given order."""
        positions = [self.index_of(pattern) for pattern in patterns]
        assert positions == sorted(positions), (
            "commands ran out of order: "
            + ", ".join(f"{p}@{i}" for p, i in zip(patterns, positions, strict=False))
            + "\nran:\n  "
            + "\n  ".join(self.rendered)
        )


class RecordingJournal:
    """A `Journal` that keeps what was reported, so a test can read it back.

    Shared rather than re-declared per test file: three of them had grown their
    own, and each widening of the protocol had to find all three.
    """

    def __init__(self) -> None:
        self.notes: list[str] = []
        self.artifacts: list[tuple[Path, str, int]] = []
        self.steps: list[str] = []
        self.actions: list[str] = []
        self.shapes: list[tuple[tuple[str, ...], tuple[tuple[str, str], ...]]] = []
        self.execs: list[dict] = []
        self.skips: list[str] = []

    def note(self, message: str) -> None:
        self.notes.append(message)

    def exec(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        exit: int,
        duration_ms: float,
    ) -> None:
        self.execs.append(
            {
                "argv": argv,
                "cwd": cwd,
                "env": env,
                "exit": exit,
                "duration_ms": duration_ms,
            }
        )

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        self.artifacts.append((path, digest, size))

    def shape(self, steps: tuple[str, ...], edges: tuple[tuple[str, str], ...]) -> None:
        self.shapes.append((steps, edges))

    def skipped(self, label: str) -> None:
        self.skips.append(label)

    @contextmanager
    def step(self, step) -> Iterator[None]:
        self.steps.append(step.label)
        yield

    @contextmanager
    def action(self, action) -> Iterator[None]:
        self.actions.append(action.render())
        yield

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
from contextlib import contextmanager, suppress
from functools import cache
from pathlib import Path
from typing import TextIO

from capsem.gate.invocation import Command
from capsem.gate.proc import Runner

PROJECT_ROOT = Path(__file__).resolve().parents[2]


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

    def launch(
        self,
        argv: Iterable[str],
        *,
        env: dict[str, str] | None = None,
        cwd: Path | None = None,
        secret_env: frozenset[str] = frozenset(),
    ) -> int:
        """Record a detached start instead of spawning one.

        Inherited from `Runner` until now, so any test that reached a `Launch`
        really did `Popen` a daemon -- which on a checkout without the binary
        built is a `FileNotFoundError` from a destructor, and on one with it is
        a stray process.
        """
        self.commands.append(
            Command(
                argv=tuple(str(part) for part in argv),
                cwd=cwd,
                env=dict(env or {}),
                secret_env=secret_env,
            )
        )
        return 424242

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
        self.launches: list[dict] = []
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

    def launch(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        pid: int,
        duration_ms: float,
    ) -> None:
        self.launches.append(
            {"argv": argv, "cwd": cwd, "env": env, "pid": pid, "duration_ms": duration_ms}
        )

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        self.artifacts.append((path, digest, size))

    def shape(self, steps: tuple[str, ...], edges: tuple[tuple[str, str], ...]) -> None:
        self.shapes.append((steps, edges))

    def skipped(self, label: str) -> None:
        self.skips.append(label)

    def step_output(self) -> Path | None:
        """Nothing: a recording journal keeps events, not bytes."""
        return None

    @contextmanager
    def step(self, step) -> Iterator[None]:
        self.steps.append(step.label)
        yield

    @contextmanager
    def action(self, action) -> Iterator[None]:
        self.actions.append(action.render())
        yield


# ---------------------------------------------------------------------------
# Reading a contract off the gate instead of off a recipe
# ---------------------------------------------------------------------------
#
# Dozens of contracts asserted against `justfile` text, because that is where
# the work was. The recipes are one-line dispatches now and the work is a plan,
# so the same claims are read by running the plan against a recording runner
# and asking what it would have issued.
#
# Cached: building and walking a plan costs seconds, and a suite that asks the
# same question thirty times should pay once.

#: Running the whole gate's plan stops at the first step that needs a real
#: machine, so "what does the gate run" is gathered per module instead -- the
#: same work, reached without one failure hiding the rest.
WHOLE_GATE: tuple[tuple[str, dict[str, object]], ...] = (
    ("candidate", {}),
    ("test-fast", {}),
    ("test-static", {}),
    ("test-artifacts", {}),
    ("test-functional", {}),
    ("test-glowup", {}),
    ("cross-compile", {"arch": "arm64"}),
    ("cross-compile", {"arch": "x86_64"}),
    ("linux-rust", {}),
    ("host-sbom", {}),
    ("install", {}),
    ("assets", {}),
)


def _built(root: Path, name: str, args: tuple[tuple[str, object], ...], qualification=None):
    import argparse

    from capsem.gate import cli  # noqa: F401 - importing registers every command
    from capsem.gate.command import GateCommand

    return GateCommand.registry[name](
        RecordingRunner(root),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **dict(args)),
        qualification=qualification,
    )


def gate_plan(name: str = "candidate", root: Path | None = None, qualification=None):
    """A command's plan, built but not run -- for asserting on its edges.

    Deliberately not cached. A plan is a mutable object with an environment in
    its inputs, and a cache keyed on the name alone hands two callers the same
    one -- so a test that set a release variable and asked again got the local
    lane's plan and asserted happily against it. Building one costs
    milliseconds; the answers it hides cost hours.
    """
    return _built(root or PROJECT_ROOT, name, (), qualification)._describe()


def gate_labels(name: str = "candidate", root: Path | None = None) -> tuple[str, ...]:
    """Every step of a command's plan, in an order the graph permits."""
    return tuple(gate_plan(name, root).labels)


def gate_issued(
    name: str, args: tuple[tuple[str, object], ...] = (), root: Path | None = None
) -> str:
    """Every command one gate command would actually run, with real argv.

    The plan is *run* against a recording runner rather than described: much of
    this work is still behind `Call`, which renders as prose, and these
    contracts are about the arguments underneath.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.context import Context

    root = root or PROJECT_ROOT
    command = _built(root, name, args)
    runner = command._runner
    try:
        plan = command._describe()
    except Exception as exc:
        return f"<plan for {name} unavailable: {exc}>"

    rendered = plan.describe()
    # A step that needs a machine fails here; what it issued before failing is
    # still the evidence.
    with suppress(Exception):
        plan.run(Context(runner, gate_config.load(root)))
    return "\n".join([rendered, *runner.rendered, *runner.notes])


def gate_issues(name: str | None = None, root: Path | None = None) -> str:
    """Everything the gate would issue, with real argv.

    `name` reads one command; the default reads the whole gate, which is what a
    contract about "does the gate ever run X" is really asking.

    Cached, unlike `gate_plan`: this runs twelve module plans and the answer is
    an immutable string. The release state is part of the key rather than
    ambient, because it changes the answer -- which is what a cache with only
    the name in its key was quietly denying.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.qualification import Qualification

    mode = Qualification.from_environment(gate_config.load(root or PROJECT_ROOT)).mode
    return _issues(name, root, mode)


@cache
def _issues(name: str | None, root: Path | None, mode: object) -> str:
    selection = (
        tuple(entry for entry in WHOLE_GATE if entry[0] == name) if name is not None else WHOLE_GATE
    )
    return "\n".join(
        gate_issued(command, tuple(sorted(args.items())), root) for command, args in selection
    )

"""Running commands, and recording which ones ran in what order.

Every recipe extracted into this package spends most of its body invoking other
programs, so the order of those invocations *is* the behaviour under test. The
`_gate-install` ordering defect -- handing the installer a manifest URL before
anything had written that manifest -- is not visible in any single command; it
is visible only in the sequence.

`Runner` therefore funnels every invocation through one overridable method. In
the gate it runs the command; in a unit test a subclass records it and answers
with canned output, so a test can assert that staging precedes the handoff
without Docker, a package, or a network.
"""

from __future__ import annotations

import os
import shlex
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import TextIO

from .errors import GateError


@dataclass(frozen=True)
class Command:
    """One invocation, in the form the runner will execute it."""

    argv: tuple[str, ...]
    cwd: Path | None = None
    env: dict[str, str] = field(default_factory=dict)
    """Additions to the inherited environment, not a replacement for it."""
    capture: bool = False
    check: bool = True

    def __str__(self) -> str:
        assignments = " ".join(
            f"{name}={shlex.quote(value)}" for name, value in sorted(self.env.items())
        )
        return f"{assignments} {shlex.join(self.argv)}".strip()


class Runner:
    """Executes gate commands against the real machine.

    Subclass and override `execute` to observe or simulate them instead.
    """

    def __init__(self, root: Path, *, stream: TextIO | None = None) -> None:
        self.root = Path(root)
        self._stream: TextIO = stream if stream is not None else sys.stderr

    # -- reporting ---------------------------------------------------------

    def step(self, message: str) -> None:
        """Announce a phase boundary in the gate's own output."""
        print(f"=== {message} ===", file=self._stream, flush=True)

    def note(self, message: str) -> None:
        print(message, file=self._stream, flush=True)

    # -- execution ---------------------------------------------------------

    def execute(self, command: Command) -> subprocess.CompletedProcess[str]:
        """The single point every invocation passes through."""
        environment = {**os.environ, **command.env}
        return subprocess.run(
            list(command.argv),
            cwd=str(command.cwd) if command.cwd else str(self.root),
            env=environment,
            check=False,
            text=True,
            stdout=subprocess.PIPE if command.capture else None,
            stderr=subprocess.PIPE if command.capture else None,
        )

    def run(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        check: bool = True,
    ) -> int:
        """Run a command, streaming its output. Returns the exit status."""
        command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            check=check,
        )
        completed = self.execute(command)
        if check and completed.returncode != 0:
            raise GateError(f"command failed ({completed.returncode}): {command}")
        return completed.returncode

    def capture(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        check: bool = True,
    ) -> str:
        """Run a command and return its stripped stdout."""
        command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            capture=True,
            check=check,
        )
        completed = self.execute(command)
        if check and completed.returncode != 0:
            detail = (completed.stderr or "").strip()
            raise GateError(
                f"command failed ({completed.returncode}): {command}"
                + (f"\n{detail}" if detail else "")
            )
        return (completed.stdout or "").strip()

    def succeeds(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
    ) -> bool:
        """Whether a probe command exits zero, discarding its output."""
        command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            capture=True,
            check=False,
        )
        return self.execute(command).returncode == 0

    # -- convenience -------------------------------------------------------

    def bash(
        self,
        script: str,
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        check: bool = True,
    ) -> int:
        """Run a shell fragment that is genuinely shell -- pipes, globs, `&&`.

        Reach for this only when the shell itself is the point. A fragment that
        merely spells out a command belongs in `run`, where its arguments stay
        separate values instead of becoming a quoting problem.
        """
        return self.run(["bash", "-c", script], cwd=cwd, env=env, check=check)

    def script(
        self,
        relative: str,
        *args: object,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        check: bool = True,
    ) -> int:
        """Run a checked-in Python script through the project's uv environment."""
        return self.run(
            ["uv", "run", "python", str(self.root / relative), *(str(a) for a in args)],
            cwd=cwd,
            env=env,
            check=check,
        )

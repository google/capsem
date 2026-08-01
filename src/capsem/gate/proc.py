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
from collections.abc import Iterator
from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass, field
from pathlib import Path
from typing import TextIO

from .errors import GateError

#: Set while a command is building its plan. Ambient rather than a property of
#: one runner, and that is the whole point: `release.py` built a *fresh*
#: `Runner(config.root)` inside `plan()` to capture `git rev-parse HEAD`, and a
#: seal that swapped the command's own runner never saw it. The dry run printed
#: a real revision while the recording runner observed nothing -- the machine
#: touched, invisibly, by the one operation whose entire value is not touching
#: it. A context variable is reachable from every runner however it was built.
_SEALED: ContextVar[bool] = ContextVar("capsem_gate_plan_sealed", default=False)


@contextmanager
def sealed() -> Iterator[None]:
    """Refuse every invocation for the duration.

    Wrapped around plan construction. A plan describes work; if building the
    description performs it, `--dry-run` is not merely incomplete but actively
    misleading, and the description can go stale before it is executed.
    """
    token = _SEALED.set(True)
    try:
        yield
    finally:
        _SEALED.reset(token)


def _refuse_while_sealed(argv: tuple[str, ...]) -> None:
    if not _SEALED.get():
        return
    raise GateError(
        f"building a plan ran {shlex.join(argv)}. plan() must describe work, "
        f"not perform it, or --dry-run touches the machine. Express the probe "
        f"as a read-only step in the plan instead."
    )

#: What `execute` hands back. Named here so the layers above can annotate their
#: own overrides without importing `subprocess` -- which only the modules that
#: genuinely touch the machine are allowed to do.
Completed = subprocess.CompletedProcess[str]


@dataclass(frozen=True)
class Command:
    """One invocation, in the form the runner will execute it."""

    argv: tuple[str, ...]
    cwd: Path | None = None
    env: dict[str, str] = field(default_factory=dict)
    """Additions to the inherited environment, not a replacement for it."""
    capture: bool = False
    check: bool = True
    log: Path | None = None
    """Append combined output here instead of streaming it.

    Two build lanes streaming to one terminal interleave into something nobody
    can read, so each concurrent lane writes its own log and only a failing
    lane's tail is surfaced.
    """

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

    def execute(self, command: Command) -> Completed:
        """The single point every invocation passes through."""
        environment = {**os.environ, **command.env}
        if command.log is not None:
            command.log.parent.mkdir(parents=True, exist_ok=True)
            with command.log.open("a", encoding="utf-8") as sink:
                return subprocess.run(
                    list(command.argv),
                    cwd=str(command.cwd) if command.cwd else str(self.root),
                    env=environment,
                    check=False,
                    text=True,
                    stdout=sink,
                    stderr=subprocess.STDOUT,
                )
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
        log: Path | None = None,
    ) -> int:
        """Run a command, streaming its output. Returns the exit status."""
        command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            check=check,
            log=log,
        )
        # Checked here rather than in `execute`, which subclasses replace: a
        # recording runner in a test overrides `execute` wholesale, and a seal
        # that test doubles slip past is a seal no test can prove.
        _refuse_while_sealed(command.argv)
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
        _refuse_while_sealed(command.argv)
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
        _refuse_while_sealed(command.argv)
        return self.execute(command).returncode == 0

    def launch(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        env: dict[str, str] | None = None,
        cwd: Path | None = None,
    ) -> int:
        """Start a process that outlives this call, and return its pid.

        Detached into its own session, and with no inherited descriptors: a
        daemon that keeps the gate's execution-lock fd holds the flock after
        the gate exits, and the next run blocks on a run that finished. The
        shell needed `3>&-` for that; Python closes non-inheritable
        descriptors across `exec` by default.
        """
        command = Command(
            argv=tuple(str(part) for part in argv), cwd=cwd, env=dict(env or {})
        )
        _refuse_while_sealed(command.argv)
        process = subprocess.Popen(
            list(command.argv),
            cwd=str(command.cwd) if command.cwd else str(self.root),
            env={**os.environ, **command.env},
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        return process.pid

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

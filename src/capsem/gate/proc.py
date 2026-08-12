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
import subprocess
import sys
import threading
from pathlib import Path
from typing import TextIO

from .errors import GateError
from .invocation import Command, ConsoleMode
from .planseal import _refuse_while_sealed
from .processgroup import StopPolicy
from .processgroup import run as run_foreground
from .processgroup import tee as tee_foreground

#: Concurrent steps write to one terminal. Their own logs are private, so this
#: guards the shared stream and nothing else -- a lock around the sinks would
#: make two lanes take turns for no reason.
_TERMINAL = threading.Lock()

#: What `execute` hands back. Named here so the layers above can annotate their
#: own overrides without importing `subprocess` -- which only the modules that
#: genuinely touch the machine are allowed to do.
Completed = subprocess.CompletedProcess[str]


class Runner:
    """Executes gate commands against the real machine.

    Subclass and override `execute` to observe or simulate them instead.
    """

    def __init__(
        self,
        root: Path,
        *,
        stream: TextIO | None = None,
        stop_policy: StopPolicy | None = None,
    ) -> None:
        self.root = Path(root)
        self._stream: TextIO = stream if stream is not None else sys.stderr
        self._configured_stop_policy = stop_policy

    def _stop_policy(self) -> StopPolicy:
        if self._configured_stop_policy is not None:
            return self._configured_stop_policy
        from .config import for_root

        execution = for_root(self.root).execution
        return StopPolicy(
            grace_seconds=execution.cancellation_grace_seconds,
            poll_seconds=execution.cancellation_poll_seconds,
        )

    # -- reporting ---------------------------------------------------------

    def step(self, message: str) -> None:
        """Announce a phase boundary in the gate's own output."""
        print(f"=== {message} ===", file=self._stream, flush=True)

    def note(self, message: str) -> None:
        print(message, file=self._stream, flush=True)

    # -- execution ---------------------------------------------------------

    # -- where a command's output belongs ----------------------------------
    #
    # Two hooks with no-op defaults, so a bare `Runner` behaves exactly as it
    # did and the funnel can file output against the step that caused it
    # without any call site passing `log=`.

    def filed(self, command: Command) -> Command:
        """The command, with somewhere for its output to go. Unchanged here."""
        return command

    def tail(self, command: Command) -> str:
        """The part of a failure worth repeating in the error. None here."""
        del command
        return ""

    def execute(self, command: Command) -> Completed:
        """The single point every invocation passes through."""
        environment = {**os.environ, **command.env}
        if command.log is not None:
            return self._teed(command, command.log, environment)
        return run_foreground(
            command.argv,
            cwd=command.cwd or self.root,
            env=environment,
            capture=command.capture,
            policy=self._stop_policy(),
        )

    def _teed(self, command: Command, log: Path, environment: dict[str, str]) -> Completed:
        """Run it, keeping the output *and* showing it.

        Redirected straight into the file before, so a step's log existed only
        for the handful of lanes that asked for one and the operator saw
        nothing while they ran. Both matter: a forty-minute gate that prints
        nothing is indistinguishable from a hung one, and output that exists
        only in a terminal is output nobody can attach to a bug report.

        The cost is a pipe, so the child no longer has a TTY -- no progress
        bars, no colour by default. That is the trade this package is here to
        make: the evidence is worth more than the animation.
        """
        log.parent.mkdir(parents=True, exist_ok=True)
        with (
            # Line buffered. A step log exists to be read after something went
            # wrong, and the default 8KB block buffer meant a hard-killed run
            # left a zero-byte file -- 700 lines to the terminal, none to disk
            # -- and that `tail -f` on a running step showed nothing until the
            # step ended. One write syscall per line is the price of both.
            log.open("a", encoding="utf-8", buffering=1) as sink,
        ):

            def record(line: str) -> None:
                sink.write(line)
                # Only the terminal is serialized. Each step owns its own sink,
                # so those need no lock and must not wait behind one.
                if command.console is ConsoleMode.STREAM:
                    with _TERMINAL:
                        self._stream.write(line)
                        self._stream.flush()

            status = tee_foreground(
                command.argv,
                cwd=command.cwd or self.root,
                env=environment,
                write=record,
                policy=self._stop_policy(),
            )
        return subprocess.CompletedProcess(args=list(command.argv), returncode=status)

    def run(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        check: bool = True,
        log: Path | None = None,
        console: ConsoleMode = ConsoleMode.STREAM,
        secret_env: frozenset[str] = frozenset(),
    ) -> int:
        """Run a command, streaming its output. Returns the exit status."""
        command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            check=check,
            log=log,
            console=console,
            secret_env=secret_env,
        )
        # Checked here rather than in `execute`, which subclasses replace: a
        # recording runner in a test overrides `execute` wholesale, and a seal
        # that test doubles slip past is a seal no test can prove.
        _refuse_while_sealed(command.argv)
        command = self.filed(command)
        completed = self.execute(command)
        if check and completed.returncode != 0:
            raise GateError(
                f"command failed ({completed.returncode}): {command}{self.tail(command)}"
            )
        return completed.returncode

    def capture(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        check: bool = True,
        secret_env: frozenset[str] = frozenset(),
    ) -> str:
        """Run a command and return its stripped stdout."""
        command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            capture=True,
            check=check,
            secret_env=secret_env,
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
        secret_env: frozenset[str] = frozenset(),
    ) -> bool:
        """Whether a probe command exits zero, discarding its output."""
        command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            capture=True,
            check=False,
            secret_env=secret_env,
        )
        _refuse_while_sealed(command.argv)
        return self.execute(command).returncode == 0

    def launch(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        env: dict[str, str] | None = None,
        cwd: Path | None = None,
        secret_env: frozenset[str] = frozenset(),
    ) -> int:
        """Start a process that outlives this call, and return its pid.

        Detached into its own session, and with no inherited descriptors: a
        daemon that keeps the gate's execution-lock fd holds the flock after
        the gate exits, and the next run blocks on a run that finished. The
        shell needed `3>&-` for that; Python closes non-inheritable
        descriptors across `exec` by default.
        """
        command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            secret_env=secret_env,
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

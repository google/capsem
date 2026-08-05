"""The invariants that wrap every command, kept where none can be forgotten.

Three rules used to live outside the code that had to obey them, and all three
were broken everywhere:

  a plan action must not start a second gate -- ten did, and because the machine
  lock is not reentrant each was a child waiting out a 7200-second timeout for
  the lock its own parent held

  every subprocess must be recorded -- `RunLog.exec` had no production caller at
  all, so no run log held a single command

  building a plan must not run anything -- the release plan captured
  `git rev-parse HEAD` while being constructed, so `--dry-run` touched the
  machine

Each was a rule a command author had to remember. Here they are properties of
the runner every invocation already passes through, so remembering is not part
of it. `GuardedRunner` wraps rather than replaces, which is what lets a
recording runner in a test be guarded exactly as the real one is -- a guard
that cannot be exercised is a guard nobody should trust.
"""

from __future__ import annotations

import shlex
import time
from dataclasses import replace
from pathlib import Path

from .context import Journal
from .errors import GateError
from .invocation import Command
from .proc import Completed, Runner
from .runlogschema import OutputSpan


def _written(log: Path | None) -> int:
    """How much a step's log already holds. Zero when there is no log yet."""
    return log.stat().st_size if log is not None and log.is_file() else 0


def _span(log: Path | None, offset: int) -> OutputSpan | None:
    """The byte range one command contributed to its step's log.

    Absent when nothing filed the output: a captured command is data its caller
    parses rather than narration, and a command issued outside any step has no
    step log to sit in.
    """
    if log is None or not log.is_file():
        return None
    return OutputSpan(file=log.name, offset=offset, length=log.stat().st_size - offset)


#: Programs that start a gate. Invoking one from inside a plan is the deadlock
#: the composition model exists to make unrepresentable.
ENTRYPOINTS = frozenset({"just", "capsem-gate"})

#: Programs that run another program. `uv run capsem-gate assets` runs
#: capsem-gate, so a check on `argv[0]` alone is defeated by the exact spelling
#: the gate already used at every one of these call sites.
WRAPPERS = frozenset({"uv", "caffeinate", "env", "nohup", "time", "stdbuf"})


def program(argv: tuple[str, ...]) -> str:
    """The program this argv actually runs, seen through any wrappers.

    Deliberately not a scan for the word anywhere in argv: `docker run --label
    just alpine` runs docker, and a check that flagged it would be deleted
    within a day -- taking the real rule with it.
    """
    index = 0
    while index < len(argv):
        name = Path(argv[index]).name
        if name not in WRAPPERS:
            return name
        index += 1
        # A wrapper's own flags, its `run` subcommand, and `VAR=value`
        # assignments all sit between it and the program it wraps.
        while index < len(argv) and (
            argv[index].startswith("-") or "=" in argv[index] or argv[index] == "run"
        ):
            index += 1
    return Path(argv[0]).name if argv else ""


def _refuse_reentry(argv: tuple[str, ...]) -> None:
    name = program(argv)
    if name not in ENTRYPOINTS:
        return
    raise GateError(
        f"this run invoked {name!r}, which starts a second gate: "
        f"{shlex.join(argv)}. The machine lock is not reentrant, so that child "
        f"would wait out its timeout for the lock this run is holding. Compose "
        f"the other command's fragment into this plan instead."
    )


class GuardedRunner(Runner):
    """Refuses gate re-entry, records everything it runs, and keeps its output."""

    def __init__(self, inner: Runner, *, journal: Journal, tail_lines: int = 0) -> None:
        super().__init__(inner.root)
        self._inner = inner
        self._journal = journal
        self._tail_lines = tail_lines

    def filed(self, command: Command) -> Command:
        """Send output to the log of whichever step is running.

        `RunLog.step_log` existed and nothing called it, so a real recorded
        release run had a `steps/` directory with no files in it. Attached here
        rather than at the call sites, for the same reason the recording is:
        there are hundreds of them and one funnel.

        Captured output is exempt -- it is data a caller parses, not narration.
        """
        if command.log is not None or command.capture:
            return command
        active = self._journal.step_output()
        return command if active is None else replace(command, log=active)

    def tail(self, command: Command) -> str:
        """The last few lines the failing command wrote.

        The whole log stays in `steps/`; this is what has to be in front of
        whoever is reading the failure. It is the tail of the step's log, so a
        step that ran several commands shows the end of its own story -- which
        is the context you want anyway.
        """
        if command.log is None or self._tail_lines <= 0 or not command.log.is_file():
            return ""
        lines = command.log.read_text(encoding="utf-8", errors="replace").splitlines()
        kept = lines[-self._tail_lines :]
        return "\n" + "\n".join(kept) if kept else ""

    def execute(self, command: Command) -> Completed:
        _refuse_reentry(command.argv)
        # Where this command's output will start. Read before it runs, because
        # afterwards the only thing the file can say is how much is in it
        # altogether -- and a step's log is shared by every command the step
        # issues. Safe without a lock: a step runs its actions in order, and
        # concurrent steps each own a different file.
        started_at = _written(command.log)
        started = time.monotonic()
        completed = self._inner.execute(command)
        self._journal.exec(
            # The evidence forms, never the raw ones. What the command added,
            # never the ambient environment -- this log is attached to bug
            # reports and a release machine's environment holds tokens -- and
            # with declared credentials reduced to their names.
            command.evidence_argv,
            cwd=str(command.cwd or self.root),
            env=command.evidence_env,
            exit=completed.returncode,
            duration_ms=(time.monotonic() - started) * 1000,
            output=_span(command.log, started_at),
        )
        return completed

    def launch(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        env: dict[str, str] | None = None,
        cwd: Path | None = None,
        secret_env: frozenset[str] = frozenset(),
    ) -> int:
        """Detached processes are guarded too -- a daemon is still a program.

        And recorded: a daemon nobody wrote down is a daemon nobody can
        account for when it outlives the run that started it.
        """
        rendered = tuple(str(part) for part in argv)
        _refuse_reentry(rendered)
        started = time.monotonic()
        pid = self._inner.launch(argv, env=env, cwd=cwd, secret_env=secret_env)
        evidence = Command(argv=rendered, env=dict(env or {}), secret_env=secret_env)
        self._journal.launch(
            evidence.evidence_argv,
            cwd=str(cwd or self.root),
            env=evidence.evidence_env,
            pid=pid,
            duration_ms=(time.monotonic() - started) * 1000,
        )
        return pid

    # -- reporting belongs to whoever owns the terminal --------------------

    def step(self, message: str) -> None:
        self._inner.step(message)

    def note(self, message: str) -> None:
        self._inner.note(message)

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
from pathlib import Path

from .context import Journal
from .errors import GateError
from .proc import Command, Completed, Runner

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
        f"a plan action invoked {name!r}, which starts a second gate: "
        f"{shlex.join(argv)}. The machine lock is not reentrant, so that child "
        f"would wait out its timeout for the lock this run is holding. Compose "
        f"the other command's fragment into this plan instead."
    )


class GuardedRunner(Runner):
    """Refuses gate re-entry, and records everything it does run."""

    def __init__(self, inner: Runner, *, journal: Journal) -> None:
        super().__init__(inner.root)
        self._inner = inner
        self._journal = journal

    def execute(self, command: Command) -> Completed:
        _refuse_reentry(command.argv)
        started = time.monotonic()
        completed = self._inner.execute(command)
        self._journal.exec(
            command.argv,
            # What the command added, never the ambient environment: this log
            # is attached to bug reports and a release machine's environment
            # holds tokens.
            cwd=str(command.cwd or self.root),
            env=dict(command.env),
            exit=completed.returncode,
            duration_ms=(time.monotonic() - started) * 1000,
        )
        return completed

    def launch(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        env: dict[str, str] | None = None,
        cwd: Path | None = None,
    ) -> int:
        """Detached processes are guarded too -- a daemon is still a program."""
        _refuse_reentry(tuple(str(part) for part in argv))
        return self._inner.launch(argv, env=env, cwd=cwd)

    # -- reporting belongs to whoever owns the terminal --------------------

    def step(self, message: str) -> None:
        self._inner.step(message)

    def note(self, message: str) -> None:
        self._inner.note(message)

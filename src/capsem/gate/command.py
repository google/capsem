"""One `capsem-gate` subcommand, with its lifecycle already decided.

A command declares two things: what it holds, and what work it contains. It
does not decide when to release, in what order steps run, what happens when one
raises, whether it needs the machine to itself, or how any of it is recorded.
That is `execute`, and it is the same for every command -- which is the point,
and why a contract test forbids overriding it.

Before this, `cli.py` called a bare handler with no `finally` anywhere near it.
Every resource a command held was that command's private problem, re-solved per
file and got subtly wrong in at least two of them. `--dry-run` could not have
existed at all, because there was nothing that knew what a command was about to
do without doing it.
"""

from __future__ import annotations

import argparse
from abc import ABC, abstractmethod
from typing import ClassVar

from . import config as gate_config
from .context import Context
from .lifecycle import Resource, held
from .locks import ExclusiveLock
from .plan import Plan
from .proc import Runner
from .runhistory import read
from .runlog import RunLog
from .timing import measure, report


class GateCommand(ABC):
    """A subcommand. Subclasses declare what they hold and what they do."""

    name: ClassVar[str]
    help: ClassVar[str]

    exclusive: ClassVar[bool] = False
    """Whether this needs the machine to itself.

    True for anything that wipes `CAPSEM_HOME`, drives Docker storage rails, or
    starts a service -- which is to say every gate proper. False for the ones
    that only read, so a developer can ask `runs show` a question while a gate
    is running.
    """

    registry: ClassVar[dict[str, type[GateCommand]]] = {}

    def __init_subclass__(cls, *, name: str, help: str, **kwargs: object) -> None:
        super().__init_subclass__(**kwargs)
        cls.name, cls.help = name, help
        if name in GateCommand.registry:
            raise TypeError(
                f"two commands claim the name {name!r}; one of them will never run"
            )
        GateCommand.registry[name] = cls

    def __init__(self, runner: Runner, args: argparse.Namespace) -> None:
        self._runner = runner
        self._args = args
        self._config = gate_config.for_root(runner.root)

    # -- what a subclass declares ------------------------------------------

    @classmethod  # noqa: B027 - concrete and empty on purpose, see below
    def add_arguments(cls, parser: argparse.ArgumentParser) -> None:
        """Declare this command's own flags.

        Deliberately concrete and deliberately empty: most commands take none,
        and forcing every one of them to write an empty override would be
        ceremony that teaches readers to skip the method that matters.
        """

    def resources(self) -> tuple[Resource, ...]:
        """What to hold for the whole command, acquired in this order.

        Released in reverse, so the order here *is* the teardown order -- which
        is why it is declared rather than written out in a `finally`.
        """
        return ()

    def reexec(self) -> tuple[str, ...] | None:
        """A command line to become, instead of running this one.

        Consulted before anything is acquired. A re-exec that happens inside
        the held resources deadlocks: the child asks for the machine lock its
        own parent is holding, and waits out the full timeout for it.

        Return `None` -- the default -- to run normally.
        """
        return None

    @abstractmethod
    def plan(self) -> Plan:
        """The work, and what must finish before what."""

    # -- what every command does the same way ------------------------------

    def execute(self) -> None:
        """Planned, locked, held, recorded, then run.

        Never overridden: `tests/test_gate_command.py` fails if a subclass
        defines it, because a command that bypasses this bypasses teardown,
        the machine lock, and the run log at once.
        """
        # Before the plan, and long before any resource: see `reexec`.
        replacement = self.reexec()
        if replacement is not None:
            raise SystemExit(self._runner.run(replacement, check=False))

        plan = self.plan()

        if self._args.graph:
            print(plan.mermaid())
            return
        if self._args.dry_run:
            print(plan.describe())
            return

        with RunLog.open(self._config, self.name, argv=self._argv()) as log:
            context = Context(self._runner, self._config, journal=log)
            with held(*self._holdings()):
                plan.run(context)
            self._summarize(log)

    def _holdings(self) -> tuple[Resource, ...]:
        """The machine lock first, then whatever the command declared.

        First because it is released last, and because the resources a command
        declares are the ones that wipe trees -- taking the lock after one of
        those has started is taking it too late.
        """
        if not self.exclusive:
            return self.resources()
        lock = ExclusiveLock.for_gate(self._config, purpose=self._purpose())
        return (lock, *self.resources())

    def _summarize(self, log: RunLog) -> None:
        """Say where the time went, on the way out."""
        if not self._args.timing:
            return
        timing = measure(read(log.directory, self._config.runlog))
        print(
            report(
                timing,
                command=self.name,
                settings=self._config.runlog,
                run_id=log.run_id,
            )
        )

    def _argv(self) -> tuple[str, ...]:
        return (self.name, *getattr(self._args, "argv", ()))

    def _purpose(self) -> str:
        """What contention should call this, for whoever arrives next."""
        return f"capsem-gate {self.name}"

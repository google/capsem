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
import os
import sys
from abc import ABC, abstractmethod
from typing import ClassVar

from . import config as gate_config
from . import prefix
from .context import Context
from .errors import GateError
from .funnel import GuardedRunner
from .lifecycle import Resource, environment_of, held
from .locks import ExclusiveLock
from .observing import observing
from .plan import Plan
from .planseal import sealed
from .proc import Runner
from .qualification import Qualification
from .qualification import from_environment as qualification_for
from .recording import Recorded


class GateCommand(Recorded, ABC):
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

    private_checkout: ClassVar[bool] = False
    """Whether this runs from a private copy of the checkout instead of it.

    True for anything long enough that someone will edit the tree while it
    runs, which in practice is every gate proper. Four release runs have died
    to exactly that, the last after 61 minutes -- and the observer had already
    named the intruding file 23 minutes before the run noticed, which is what
    settles that detection is not the fix.

    Declared per command rather than inferred, because the copy is not free:
    the run starts with no `target/`, so a command too short to be raced pays
    a cold build to avoid a race it was never going to lose.
    """

    uses_qualification: ClassVar[bool] = False
    """Whether this command's plan depends on which artifacts a release chose.

    False for everything that only reports. A half-exported release
    environment is refused -- it can only produce a hybrid proof -- but that
    refusal must not reach `runs last`, `logs` or `gc --dry-run`, which are
    exactly what an operator reaches for *because* the workflow broke. Declared
    rather than inferred: guessing from the name or from what the plan happens
    to mention puts the answer somewhere nobody looks when adding a module.
    """

    registry: ClassVar[dict[str, type[GateCommand]]] = {}

    def __init_subclass__(cls, *, name: str, help: str, **kwargs: object) -> None:
        super().__init_subclass__(**kwargs)
        cls.name, cls.help = name, help
        if name in GateCommand.registry:
            raise TypeError(f"two commands claim the name {name!r}; one of them will never run")
        GateCommand.registry[name] = cls

    def __init__(
        self,
        runner: Runner,
        args: argparse.Namespace,
        *,
        qualification: Qualification | None = None,
        invocation: tuple[str, ...] = (),
    ) -> None:
        self._runner = runner
        self._args = args
        # What was typed, captured by `cli` before parsing. A run that cannot
        # say which channel it attempted is a run nobody can read back.
        self._invocation = invocation
        self._config = gate_config.for_root(runner.root)
        # Read once, here, so no module below decides for itself whether it is
        # in a release lane -- three did, from three different variables, and
        # nothing compared their answers. Eagerly, so a broken environment
        # stops the run before it spends an hour rather than when the module
        # that happens to look reaches it -- but only for the commands whose
        # plan depends on the answer.
        self._qualification = qualification
        if qualification is None and self.uses_qualification:
            self._qualification = qualification_for(self._config)

    @property
    def qualification(self) -> Qualification:
        """Which artifacts this run is proving.

        Only for commands that declared they depend on it. Reaching for it
        without that declaration is a wiring mistake worth naming: the state
        was never parsed, so the answer would be `None` and the plan would
        silently take the local branch on a release runner.
        """
        if self._qualification is None:
            raise GateError(
                f"{self.name} read the release qualification without declaring "
                "uses_qualification = True, so it was never parsed"
            )
        return self._qualification

    # -- what a subclass declares ------------------------------------------

    @classmethod
    def add_arguments(cls, parser: argparse.ArgumentParser) -> None:
        """Declare this command's own flags.

        Deliberately concrete and deliberately empty: most commands take none,
        and forcing every one of them to write an empty override would be
        ceremony that teaches readers to skip the method that matters.
        """

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        """What to hold for the whole command, acquired in this order.

        Released in reverse, so the order here *is* the teardown order -- which
        is why it is declared rather than written out in a `finally`.

        `runner` is the guarded, journaling one, and resources are built with
        it rather than with the command's raw runner. They were not: so the
        orphan baseline, Colima, the service launch and the failure-evidence
        capture all ran outside the funnel -- no `exec` events, and no
        re-entry refusal on the one code path that executes while the machine
        lock is held.
        """
        del runner  # a command with nothing to hold needs no runner to hold it
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
        """Planned, asserted, locked, held, recorded, then run.

        Never overridden: `tests/test_gate_command.py` fails if a subclass
        defines it, because a command that bypasses this bypasses teardown, the
        machine lock, the run log and every invariant below at once.

        The order is the contract. Each line is here because the alternative
        arrangement was tried and broke something.
        """
        # A plan describes; it does not act. Built against a runner that
        # refuses everything, so `--dry-run` cannot touch the machine on the
        # way to telling you it would not.
        plan = self._describe()
        plan.validate(self._config)

        # Inspection before re-exec. The other way round, `candidate --dry-run`
        # re-execed into a real `just test`: an inert question starting a
        # forty-minute destructive gate.
        if self._args.graph:
            print(plan.mermaid())
            return
        if self._args.dry_run:
            print(plan.describe())
            return

        # The same deadlock, reached from inside Python rather than through a
        # subprocess. `GuardedRunner` cannot see it: nothing is spawned. A
        # pytest step calling `cli.main(["storage", ...])` simply blocked on
        # the lock its own grandparent held, and the run stayed alive-looking
        # for the full two hours.
        self._refuse_inside_a_run()

        # Before the re-exec and before any resource, for the same reason both
        # of those are here: the child takes the machine lock, so a parent
        # holding it would wait out its own timeout. The copy is built by this
        # process and reclaimed by it after the child returns, which is what
        # gives the export somewhere to run even when the run failed.
        if self.private_checkout and prefix.source_checkout(self._config) is None:
            raise SystemExit(prefix.run_from_private_copy(self._runner, self._config, sys.argv[1:]))

        # Before any resource, and outside the lock: a re-exec inside the held
        # resources deadlocks, because the child asks for the lock its own
        # parent is holding and waits out the full timeout.
        replacement = self.reexec()
        if replacement is not None:
            raise SystemExit(self._runner.run(replacement, check=False))

        with self._recording() as log, observing(self._config, log, plan) as watch:
            # Every invocation from here is recorded, and none may start a
            # second gate. Neither is a call site's responsibility.
            runner = GuardedRunner(
                self._runner,
                journal=log,
                tail_lines=self._config.runlog.failure_tail_lines,
            )
            with held(*self._holdings(runner)) as acquired:
                plan.run(
                    Context(
                        runner,
                        self._config,
                        journal=log,
                        env=environment_of(acquired),
                        watch=watch,
                    )
                )
        # Outside the run log's own context, so `run.end` is on disk before
        # anything reads the run back. Inside it, `--timing` measured a run
        # that had not finished and reported `total_ms == 0`.
        self._summarize(log)

    def _refuse_inside_a_run(self) -> None:
        """Refuse to take a lock this process tree is already holding.

        Only for commands that take it. A read-only command is exactly what
        someone wants from inside a running gate -- `runs last` while it works
        is the point of `runs last`.
        """
        if not self.exclusive:
            return
        holder = os.environ.get(self._config.locks.gate.run_marker)
        if holder is None:
            return
        raise GateError(
            f"{self.name} takes the machine lock, and this process is already "
            f"inside the gate run holding it ({holder}). It would wait out its "
            "full timeout for a lock that cannot be released until it returns. "
            "Compose this command's fragment into that plan, or drive its plan "
            "directly if this is a test."
        )

    def _describe(self) -> Plan:
        """Build the plan with the machine sealed off.

        Ambient rather than a swapped-in runner: a module that builds its own
        `Runner` inside `plan()` -- which `release.py` did, to capture
        `git rev-parse HEAD` -- escapes anything scoped to this instance.
        """
        with sealed():
            return self.plan()

    def _holdings(self, runner: Runner) -> tuple[Resource, ...]:
        """The machine lock first, then whatever the command declared.

        First because it is released last, and because the resources a command
        declares are the ones that wipe trees -- taking the lock after one of
        those has started is taking it too late.
        """
        if not self.exclusive:
            return self.resources(runner)
        lock = ExclusiveLock.for_gate(self._config, purpose=self._purpose())
        return (lock, *self.resources(runner))

    def _purpose(self) -> str:
        """What contention should call this, for whoever arrives next."""
        return f"capsem-gate {self.name}"

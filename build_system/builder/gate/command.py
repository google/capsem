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
import sys
from abc import ABC, abstractmethod
from typing import ClassVar

from . import config as gate_config
from . import egress, enforcement, planreport, prefix, preflight, qualificationflow, resume, sandbox
from .commandhooks import CommandHooks
from .context import Context
from .errors import GateError
from .funnel import GuardedRunner
from .lifecycle import Resource, environment_of, held
from .observing import observing
from .plan import Plan
from .planseal import sealed
from .proc import Runner
from .qualification import Qualification
from .qualification import from_environment as qualification_for
from .qualification import is_release as qualification_is_release
from .qualificationevidence import QualificationPolicy
from .recording import Recorded
from .scopeenv import command_environment
from .sourcecommit import SourceCommit, qualified_commit


class GateCommand(CommandHooks, Recorded, ABC):
    publishes: ClassVar[bool] = False
    """Whether this command can make something other people see."""

    name: ClassVar[str]
    help: ClassVar[str]

    exclusive: ClassVar[bool] = False
    """Whether this needs the machine to itself.

    True for anything that wipes `CAPSEM_HOME`, mutates managed caches, or
    starts a service -- which is to say every gate proper. False for the ones
    that only read, so a developer can ask `runs show` a question while a gate
    is running.
    """

    sandboxed: ClassVar[sandbox.SandboxMode] = sandbox.OFF
    """Whether this command runs under the host kernel sandbox, and how.

    Off by default. `--sandbox` may override incomplete commands; complete
    qualification is checked at the first line of `execute`.
    """
    complete_qualification: ClassVar[bool] = False
    qualification_policy: ClassVar[QualificationPolicy] = QualificationPolicy.NONE
    outside_egress: ClassVar[bool] = False

    private_checkout: ClassVar[bool] = False
    """Whether this runs from a private copy of the checkout instead of it."""

    uses_qualification: ClassVar[bool] = False
    """Whether this command's plan depends on which artifacts a release chose.

    False for everything that only reports. A half-exported release
    environment is refused -- it can only produce a hybrid proof -- but that
    refusal must not reach `runs last` or `logs`, which are exactly what an
    operator reaches for *because* the workflow broke. Declared
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
        # Captured before parsing so a failed run retains its exact argv.
        self._invocation = invocation
        self._config = gate_config.for_root(runner.root)
        self._sandbox_mode = sandbox.mode(self.sandboxed, getattr(args, "sandbox", None))
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

    def source_commit(self) -> SourceCommit | None:
        return None

    def reexec(self) -> tuple[str, ...] | None:
        """A command line to become, instead of running this one.

        Consulted before anything is acquired. A re-exec that happens inside
        the held resources deadlocks: the child asks for the machine lock its
        own parent is holding, and waits out the full timeout for it.

        Module commands share the complete gate's host wrapper, so direct
        release-CI fragment invocations retain the qualification boundary."""
        return sandbox.reexec(
            self._config,
            self._runner,
            default=self._sandbox_mode,
            requested=None,
            outside_egress=self.outside_egress,
        )

    @abstractmethod
    def plan(self) -> Plan:
        """The work, and what must finish before what."""

    # -- what every command does the same way ------------------------------

    def execute(self) -> None:
        """Planned, asserted, locked, held, recorded, then run.

        Never overridden: `build_system/tests/gate/test_gate_command.py` fails if a subclass
        defines it, because a command that bypasses this bypasses teardown, the
        machine lock, the run log and every invariant below at once.

        The order is the contract. Each line is here because the alternative
        arrangement was tried and broke something.
        """
        sandbox.require_complete_qualification(
            self.name, self._sandbox_mode, enforcement.enforcement_required(self)
        )
        # A plan describes; its runner refuses work, so inspection cannot act.
        plan = self._describe()
        plan.validate(self._config)

        carried, reuse = resume.resolve(
            plan,
            self._config,
            self._args,
            qualifying=self.publishes or qualification_is_release(self._qualification),
        )

        if planreport.answered(plan, self._args, carried):
            return

        preflight.refuse_inside_a_run(self._config, self.name, exclusive=self.exclusive)
        commit = self.source_commit()
        decision = qualificationflow.decide(
            self._config,
            policy=self.qualification_policy,
            commit=commit,
            plan=plan,
            args=self._args,
            carried=carried,
            reuse_path=reuse,
        )
        if decision.shortcut and self.qualification_policy is QualificationPolicy.REUSE_OR_RUN:
            assert commit is not None
            assert decision.complete is not None
            self._record_qualification_reuse(commit, decision.complete)
            return
        carried, reuse = decision.carried, decision.reuse
        if message := qualificationflow.progress(
            decision, commit, getattr(self._args, "resume_from", None)
        ):
            self._runner.note(message)
        if (self.private_checkout or reuse) and not prefix.active(self._config, commit):
            raise SystemExit(
                prefix.run_from_private_copy(
                    self._runner,
                    self._config,
                    [*sys.argv[1:], *decision.child_arguments],
                    reuse=reuse,
                    commit=commit,
                    clean=getattr(self._args, "clean_build", False),
                )
            )

        replacement = self.reexec()
        if replacement is not None:
            raise SystemExit(self._runner.run(replacement, check=False))
        self.admit(commit)
        with (
            self._recording(source_commit=None if commit is None else str(commit)) as log,
            preflight.locked(self._config, self._runner, self.name, exclusive=self.exclusive) as locked,
            observing(self._config, log, plan, publishes=self.publishes) as watch,
        ):
            qualificationflow.begin(log, decision, commit, self.qualification_policy)
            # Every invocation from here is recorded, and none may start a
            # second gate. Neither is a call site's responsibility.
            runner = GuardedRunner(
                self._runner,
                journal=log,
                tail_lines=self._config.runlog.failure_tail_lines,
                checkpoint=None if watch is None else watch.checkpoint,
            )
            acquiring = preflight.holdings(
                self._config, runner, self.name,
                exclusive=self.exclusive,
                declared=egress.for_command(self, runner),
            )
            with held(*acquiring) as acquired:
                from .egress import guarded_runner_of

                outside_runner = guarded_runner_of(
                    acquired,
                    journal=log,
                    tail_lines=self._config.runlog.failure_tail_lines,
                    checkpoint=None if watch is None else watch.checkpoint,
                )
                plan.run(
                    Context(
                        runner,
                        self._config,
                        journal=log,
                        outside_runner=outside_runner,
                        env=command_environment(
                            self._config,
                            environment_of((*locked, *acquired)),
                            self._sandbox_mode,
                            source_commit=qualified_commit(self._config.root, commit),
                        ),
                        watch=watch,
                        carried=carried,
                    )
                )
                qualificationflow.finish(
                    log, self._config, commit, self.qualification_policy, plan, decision
                )
                self.completed(commit)

    def _describe(self) -> Plan:
        """Build the plan with the machine sealed off.

        Ambient rather than a swapped-in runner: a module that builds its own
        `Runner` inside `plan()` -- which `release.py` did, to capture
        `git rev-parse HEAD` -- escapes anything scoped to this instance.
        """
        with sealed():
            return self.plan()

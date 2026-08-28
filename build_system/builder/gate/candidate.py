"""`just test-clean`: the exceptional complete local diagnostic.

Two of this command's three guarantees cannot be steps, and understanding why
is most of the design.

**The process count happens even when the run aborts.** An aborted run is the
one that skips its own cleanup, so it is exactly the run whose surviving
processes need counting -- sixteen `capsem-service` processes, each holding a
tray, once accumulated in a day while every run reported success. A step whose
dependency failed is *skipped*, which is right for work and wrong for cleanup.
A `Resource` releases on every path, so the accounting is one.

**Colima is restored to what the developer had.** That was a shell trap in
`with-gate-colima.sh`, wrapping the expensive half of the gate. "Give back what
I found on the way in" is the resource abstraction exactly, so it is one too --
and the trap, along with the script, goes.

**The source cannot move underneath the run.** That one *is* a pair of steps,
because it must not run when the gate failed: the failure is the report.

The third hazard the shell had here is gone by construction. Inside an EXIT trap
`$?` is the *last command's* status, which on Ctrl-C is 0, so `exit "$status"`
discarded the shell's own 130 and turned an abort into a green gate. An
interrupt propagates through `held` unless something explicitly swallows it, and
`planrunner` re-raises rather than recording it as a step result.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys

from . import candidateplan, host, sandbox
from . import config as gate_config
from .command import GateCommand
from .errors import GateError
from .gateresources import Resource, gate_resources
from .plan import Plan
from .proc import Runner
from .qualificationevidence import QualificationPolicy
from .sourcecommit import SourceCommit, optional_source_commit


def keep_awake(runner: Runner) -> list[str] | None:
    """The prefix that stops macOS sleeping through an unattended gate.

    `None` once already applied, or on a platform that does not need it. A
    forty-minute run that dies at minute thirty proves nothing, and the machine
    is usually unattended by then.
    """
    settings = gate_config.for_root(runner.root).candidate
    if not host.on_macos() or os.environ.get(settings.keep_awake_marker):
        return None

    command = settings.keep_awake_command[0]
    if shutil.which(command) is None:
        raise GateError(
            f"macOS just test-clean requires {command} to prevent an unattended "
            "release gate from sleeping"
        )
    return [*settings.keep_awake_command, "env", f"{settings.keep_awake_marker}=1"]


#: What `python -m` runs to become the gate again. Spelled once: `gatelaunch`
#: uses the same target, and a re-exec that names a different one is a re-exec
#: that runs different code.
MODULE = "capsem_builder.gate"


class CompleteGate:
    """What a command that *contains* the whole gate owes the machine.

    A mixin, not a base command: a base would have to register a runnable
    name, and there is nothing here to run. Only `candidate` owns the complete
    cold diagnostic; release commands dispatch hosted qualifying lanes instead
    of inheriting this multi-hour local lifecycle.
    """

    _config: gate_config.GateConfig
    _runner: Runner
    _args: argparse.Namespace
    sandboxed: sandbox.SandboxMode = sandbox.ENFORCE
    _sandbox_mode: sandbox.SandboxMode
    """Provided by `GateCommand`; declared so the mixin type-checks alone."""

    outside_egress = False
    complete_qualification = True
    """Its success may be accepted as complete qualification.

    `GateCommand.execute` therefore requires an enforcing sandbox before it
    constructs this mixin's plan or reaches any lifecycle boundary.
    """

    private_checkout = True
    """The complete gate reads a copy of the checkout, never the checkout.

    Here rather than on `candidate`, because this mixin *is* the set of
    commands that spend the multi-hour proof -- which is the same set long
    enough for someone to edit the tree while it runs.

    It is not free: a private copy starts with no `target/`, and `test-fast`
    measures 89s from a prefix against 28s in a warm checkout. That ratio is
    the price of a qualification whose subject cannot move while it runs, and
    only a command that already costs an hour should pay it.

    Publication has its own short detached prefix and does not consume this
    machine-local diagnostic journal.
    """

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return gate_resources(
            self._config,
            runner,
            mode=self._sandbox_mode,
            outside_egress=self.outside_egress,
        )

    def reexec(self) -> tuple[str, ...] | None:
        """Become *this* command under its host wrappers, once.

        Before the machine lock, not inside it. As a step this deadlocked --
        the re-exec'd child asked for the lock its own parent was holding and
        waited out the two-hour timeout. `keep_awake` returns None on the
        second pass, so this happens exactly once.

        The replacement is the operator's own arguments, not a recipe.
        Returning the recipe dropped whatever flags they passed and sent an
        already-dispatched command back through the dispatch chain -- a wrapper
        should wrap the thing it was given, not substitute something that
        usually arrives at the same place.

        The *program* is this interpreter running this module, not `sys.argv[0]`.
        `capsem-gate` re-execs itself under an isolated bytecode cache with
        `-m capsem_builder.gate`, so from here `sys.argv[0]` is the path of
        `__main__.py` -- a file that is not executable. Passing it to
        `caffeinate` gave `env: __main__.py: Permission denied` and a gate that
        stopped in three seconds.
        """
        awake_wrapper = keep_awake(self._runner)
        sandbox_mode = self._sandbox_mode
        needs_sandbox = sandbox_mode != sandbox.OFF and not sandbox.active(self._config)
        if awake_wrapper is None and not needs_sandbox:
            return None
        replacement = (sys.executable, "-m", MODULE, *sys.argv[1:])
        if awake_wrapper is not None:
            self._runner.step("Holding macOS awake for the complete gate")
            replacement = (*awake_wrapper, *replacement)
        # Under the sandbox too, when this run asked for one -- here rather
        # than anywhere later, because a profile is inherited by every child
        # and cannot be dropped. See `sandbox.applied`.
        if needs_sandbox:
            if self.outside_egress:
                sandbox.prepare_egress(self._config)
            return sandbox.applied(
                self._config,
                self._runner,
                default=sandbox_mode,
                requested=None,
                argv=replacement,
            )
        return replacement


class CandidateCommand(
    CompleteGate,
    GateCommand,
    name="candidate",
    help="run the complete local qualification gate",
):
    exclusive = True
    uses_qualification = True
    outside_egress = True
    qualification_policy = QualificationPolicy.REUSE_OR_RUN
    """Phase 8b: the complete local gate refuses the network.

    `candidate` only. Release has a small networked dispatch plan, but consumes
    the enforcing candidate journal rather than rerunning qualification with a
    wider profile.

    Complete qualification accepts only enforcement. Run an individual module
    in report mode to measure a rule without creating qualification evidence.
    """

    # The run this whole mechanism was built for. `just test-clean` is this command,
    # and it composes the modules' plan *fragments* in-process rather than
    # invoking their commands -- so declaring `private_checkout` on the modules
    # protects `capsem-gate test-fast` typed by hand and does nothing for the
    # hour-long qualification, which is the one that has died four times. It
    # arrives here from `CompleteGate`.

    @classmethod
    def add_arguments(cls, parser: argparse.ArgumentParser) -> None:
        parser.add_argument("source_commit", nargs="?", type=optional_source_commit)

    def source_commit(self) -> SourceCommit | None:
        return getattr(self._args, "source_commit", None)

    def plan(self) -> Plan:
        plan = Plan(self.name)
        candidateplan.compose(plan, self._config, qualification=self.qualification)
        return plan


class CandidateModulesCommand(
    GateCommand,
    name="test-candidate",
    help="every checked-in module, after rebuilding the assets they run against",
):
    """The gate minus its fast phase, for when that phase already passed.

    Shares `candidateplan` with `candidate` rather than launching the four
    module commands as separate processes, which is what it used to do -- four
    exclusive children, each waiting for the lock this command was holding.
    """

    exclusive = True
    uses_qualification = True

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return gate_resources(
            self._config,
            runner,
            mode=self._sandbox_mode,
        )

    def plan(self) -> Plan:
        plan = Plan(self.name)
        candidateplan.compose_modules(plan, self._config, qualification=self.qualification)
        return plan

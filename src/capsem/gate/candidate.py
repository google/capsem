"""`just test`: the complete local proof, as one process holding one lock.

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

import json
import os
import shutil
import sys

from . import candidateplan, host
from . import config as gate_config
from .command import GateCommand
from .errors import GateError
from .lifecycle import Resource
from .plan import Plan
from .proc import Runner
from .storage import Storage
from .workspace import Workspace


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
            f"macOS just test requires {command} to prevent an unattended "
            "release gate from sleeping"
        )
    return [*settings.keep_awake_command, "env", f"{settings.keep_awake_marker}=1"]


class OrphanAccounting(Resource, name="orphan-accounting"):
    """Count the capsem processes this gate leaves behind.

    Acquired first, so the baseline is taken before anything can spawn a
    process and a developer's own dev daemon is never blamed on this run.
    Released last, after the workspace has stopped its service, so what is
    still alive at that point really is a leak.
    """

    def __init__(self, config, runner: Runner) -> None:
        self._settings = config.candidate
        self._runner = runner

    def _orphan(self, action: str, *, check: bool = True) -> int:
        return self._runner.script(self._settings.orphan_script, action, check=check)

    def acquire(self) -> None:
        self._orphan("baseline")

    def release(self) -> None:
        if self._orphan("check", check=False) != 0:
            raise GateError(
                "capsem processes from this checkout outlived the gate; see above"
            )


class FailureEvidence(Resource, name="failure-evidence"):
    """Keep what a failed gate leaves behind, labelled with what it was testing.

    `preserve` runs only on failure and only before release, which is the whole
    reason it is a separate phase: release is what destroys the evidence.

    The label comes from the recorded source state rather than from a value
    captured while the plan was built, so it names the revision the run
    actually qualified.
    """

    def __init__(self, config, runner: Runner) -> None:
        self._config = config
        self._runner = runner

    def acquire(self) -> None:
        """Nothing to take: this exists for what it does on the way out."""

    def release(self) -> None:
        """Nothing to give back either."""

    def preserve(self, error: BaseException) -> None:
        Storage(self._runner).capture_failure(
            rail=self._config.candidate.failure_rail, label=self._label()
        )

    def _label(self) -> str:
        recorded = self._config.path(self._config.candidate.source_state_file)
        if not recorded.is_file():
            return self._config.candidate.unknown_head
        return json.loads(recorded.read_text(encoding="utf-8"))["head"][:12]


class Colima(Resource, name="colima"):
    """Leave Colima as the developer had it.

    If it was already running, the gate leaves it alone; if bootstrap started
    it, this stops it -- on success and on failure alike. That was a shell trap
    around the expensive half of the gate, which is to say it was correct only
    for the commands that happened to sit inside the wrapper.
    """

    def __init__(self, config, runner: Runner) -> None:
        self._settings = config.candidate
        self._runner = runner
        self._was_running = False

    def _available(self) -> bool:
        return shutil.which(self._settings.colima) is not None

    def _running(self) -> bool:
        return self._available() and self._runner.succeeds([self._settings.colima, "status"])

    def acquire(self) -> None:
        self._was_running = self._running()

    def release(self) -> None:
        if self._was_running or not self._running():
            return
        self._runner.step("Stopping gate-owned Colima VM")
        if self._runner.run([self._settings.colima, "stop"], check=False) != 0:
            self._runner.note("WARNING: failed to stop Colima started by this gate")


def gate_resources(config, runner: Runner) -> tuple[Resource, ...]:
    """What anything running the complete gate must hold.

    Order is the guarantee: acquired left to right, released in reverse. The
    orphan baseline is first taken and last compared, and the workspace stops
    its service before that comparison happens -- so what is still alive by
    then really is a leak.

    Shared with the release commands, which run the gate in-process now rather
    than launching it, and therefore hold exactly what it holds.
    """
    return (
        OrphanAccounting(config, runner),
        FailureEvidence(config, runner),
        Workspace(config),
        Colima(config, runner),
    )


class CompleteGate:
    """What a command that *contains* the whole gate owes the machine.

    A mixin, not a base command: a base would have to register a runnable
    name, and there is nothing here to run. It exists because keep-awake used
    to belong to `candidate` when the gate belonged to `candidate` -- the
    release commands reached it by launching `just test`. Deleting that child
    was right; it left them owning the same forty-minute qualification with
    none of the wrapper, so an unattended macOS release could sleep through
    its own publication.
    """

    _config: gate_config.GateConfig
    _runner: Runner

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return gate_resources(self._config, runner)

    def reexec(self) -> tuple[str, ...] | None:
        """Become *this* command under a keep-awake wrapper, once.

        Before the machine lock, not inside it. As a step this deadlocked --
        the re-exec'd child asked for the lock its own parent was holding and
        waited out the two-hour timeout. `keep_awake` returns None on the
        second pass, so this happens exactly once.

        The replacement is the operator's own argv, not a recipe. Returning
        the recipe dropped whatever flags they passed and sent an already-
        dispatched command back through the dispatch chain -- a wrapper should
        wrap the thing it was given, not substitute something that usually
        arrives at the same place.
        """
        prefix = keep_awake(self._runner)
        if prefix is None:
            return None
        self._runner.step("Holding macOS awake for the complete gate")
        return (*prefix, *sys.argv)


class CandidateCommand(
    CompleteGate,
    GateCommand,
    name="candidate",
    help="run the complete local qualification gate",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        candidateplan.compose(plan, self._config)
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

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return gate_resources(self._config, runner)

    def plan(self) -> Plan:
        plan = Plan(self.name)
        candidateplan.compose_modules(plan, self._config)
        return plan

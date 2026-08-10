"""What anything running the complete gate holds, and gives back.

Split from `candidate` at the module ceiling, along the seam that was already
there: these are lifetimes, not plans. Each exists because a step could not
express it -- a step whose dependency failed is *skipped*, which is right for
work and exactly wrong for cleanup, and an aborted run is the one whose
leftovers most need accounting for.

Order is the guarantee. Acquired left to right and released in reverse, so the
orphan baseline is first taken and last compared, and the workspace stops its
service before that comparison happens -- which is what makes a process still
alive by then genuinely a leak rather than one of ours mid-shutdown.
"""

from __future__ import annotations

import json
import shutil

from .egress import Egress
from .errors import GateError
from .lifecycle import Resource
from .proc import Runner
from .sandboxreport import SandboxReport
from .storage import Storage
from .workspace import Workspace


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
            raise GateError("capsem processes from this checkout outlived the gate; see above")


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


def gate_resources(
    config, runner: Runner, *, mode: str, outside_egress: bool = False
) -> tuple[Resource, ...]:
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
        # Second, so it is acquired before any step and released after the
        # workspace and Colima have finished theirs: what the sandbox was
        # asked for during teardown is as much a part of the allow-list as
        # what it was asked for during the run.
        SandboxReport(config, runner, mode=mode),
        Egress(config, enabled=outside_egress and mode != "off"),
        FailureEvidence(config, runner),
        Workspace(config),
        Colima(config, runner),
    )

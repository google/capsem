"""`just test`: the complete local proof, and what it guarantees on the way out.

Three guarantees, and each was subtle enough in shell to have been got wrong.

**The source cannot move underneath the run.** A forty-minute gate that
qualified a HEAD nobody has, or a working tree edited halfway through, has
proved something about no particular version of the software. Both are captured
before and compared after.

**The count happens even when the run aborts.** An aborted run is the one that
skips its own cleanup, so it is exactly the run whose surviving processes need
counting -- sixteen `capsem-service` processes, each holding a tray, once
accumulated in a day while every run reported success.

**An interrupted run is never reported as a pass.** This is the one the shell
made genuinely hard. Inside an EXIT trap `$?` is the *last command's* status,
which on Ctrl-C is 0, so `exit "$status"` discarded the shell's own 130 and
turned an abort into a green gate. `try`/`finally` has no equivalent trap: the
exception propagates unless something explicitly swallows it, and a leak raises
on its own account.
"""

from __future__ import annotations

import os
import shutil

from . import config as gate_config
from . import host
from .actions import Call
from .command import GateCommand
from .errors import GateError
from .execution import step
from .plan import Plan
from .proc import Runner
from .storage import Storage


class CandidateGate:
    """One complete local qualification run."""

    def __init__(self, runner: Runner) -> None:
        self._runner = runner
        self._config = gate_config.for_root(runner.root)
        self._settings = self._config.candidate
        self._storage = Storage(runner)

    # -- the source state under test ---------------------------------------

    def _head(self) -> str:
        return self._runner.capture(["git", "rev-parse", "HEAD"])

    def _source_digest(self) -> str:
        return self._runner.capture(
            ["uv", "run", "python", str(self._config.path(self._settings.source_digest_script))]
        )

    def _require_unchanged(self, head: str, digest: str) -> None:
        """Whatever passed must be what was measured, at both granularities."""
        if self._head() != head:
            raise GateError("source HEAD changed while just test was running")

        after = self._source_digest()
        if after != digest:
            self._runner.note(f"before={digest} after={after}")
            self._runner.run(["git", "status", "--short"], check=False)
            raise GateError("just test changed the source working tree")

    # -- process accounting ------------------------------------------------

    def _orphan(self, action: str, *, check: bool = True) -> int:
        return self._runner.script(self._settings.orphan_script, action, check=check)

    def _close_out(self, head: str) -> None:
        """Count what is still alive, and preserve evidence from a failure.

        Runs on every path, including the aborted one. A leak raises here on
        its own account, so a run that would otherwise have passed still fails
        for the processes it left behind.
        """
        if self._orphan("check", check=False) != 0:
            raise GateError(
                "capsem processes from this checkout outlived the gate; see above"
            )

    # -- the run -----------------------------------------------------------

    def run(self) -> None:
        head = self._head()
        digest = self._source_digest()
        self._runner.step(f"Testing source state {digest} at {head}")

        # Before anything can spawn a capsem process, so a developer's own dev
        # daemon or editor MCP is never blamed on this run.
        self._orphan("baseline")

        failure: BaseException | None = None
        try:
            self._runner.run(["just", self._settings.fast_module])
            self._runner.run(
                [
                    "bash",
                    str(self._config.path(self._settings.colima_wrapper)),
                    "just",
                    self._settings.candidate_module,
                ]
            )
            self._require_unchanged(head, digest)
        except BaseException as error:
            failure = error
            self._storage.capture_failure(rail="default", label=head[:12])
            raise
        finally:
            # Never lowers the status: an exception in flight keeps propagating,
            # and a leak raises on its own. The shell equivalent had to be
            # written as `return "$status"` rather than `exit "$status"`,
            # because `$?` inside a trap is the last command's -- 0 on Ctrl-C.
            try:
                self._close_out(head)
            except GateError:
                if failure is None:
                    raise

        self._runner.step(f"Verified source state {digest}")


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


class CandidateCommand(
    GateCommand, name="candidate", help="run the complete local qualification gate"
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(step("qualify", Call("the complete local gate", _qualify)))
        return plan


def _qualify(context) -> None:
    prefix = keep_awake(context.runner)
    if prefix is not None:
        # Re-exec under the keep-awake wrapper rather than holding an assertion
        # open across the whole run.
        context.runner.step("Holding macOS awake for the complete candidate gate")
        context.runner.run([*prefix, "just", "test"], check=False)
        return

    CandidateGate(context.runner).run()

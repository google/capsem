"""One memorable spelling for the gate modules developers rerun directly.

This command is intentionally an alias, not another test plan.  Each focus
group becomes the existing command that already owns its plan, resources,
sandbox, journal, and reusable products.  The alias process exits before the
target takes the machine lock, so it cannot create the nested-lock failure the
gate forbids inside plan actions.
"""

from __future__ import annotations

import argparse

from . import bench, module_rehearsal, staticmodule, vmmodules
from .command import GateCommand
from .lifecycle import Resource
from .plan import Plan
from .proc import Runner
from .qualification import Qualification

TARGETS: dict[str, type[GateCommand]] = {
    "assets": vmmodules.ArtifactsModule,
    "benchmark": bench.BenchCommand,
    "binaries": staticmodule.StaticModule,
    "functional": vmmodules.FunctionalModule,
    "install": vmmodules.GlowupModule,
    "release-system": module_rehearsal.RehearsalModule,
}


class FocusTestCommand(
    GateCommand,
    name="focus-test",
    help="rerun one owned functional group without starting qualification",
):
    """Validate a group, show its exact plan, then become its owning command."""

    uses_qualification = True

    def __init__(
        self,
        runner: Runner,
        args: argparse.Namespace,
        *,
        qualification: Qualification | None = None,
        invocation: tuple[str, ...] = (),
    ) -> None:
        args.clean_build = args.mode == "clean" or args.clean_build
        super().__init__(
            runner,
            args,
            qualification=qualification,
            invocation=invocation,
        )
        target = self._target()
        # The alias adopts the existing owner's lifecycle. It does not widen
        # the sandbox, create a private checkout for a benchmark, or omit one
        # for a VM module merely because all groups share one public spelling.
        # These declarations are ClassVars on ordinary commands because their
        # owner is fixed. This alias selects its owner from a closed argument,
        # so its instance must shadow those declarations before `execute`
        # consults them. Updating the instance dictionary makes that dynamic
        # boundary explicit without pretending the class has one lifecycle.
        self.__dict__.update(
            exclusive=target.exclusive,
            outside_egress=target.outside_egress,
            private_checkout=target.private_checkout,
        )
        self._sandbox_mode = target._sandbox_mode

    @classmethod
    def add_arguments(cls, parser: argparse.ArgumentParser) -> None:
        parser.add_argument("group", choices=tuple(TARGETS))
        parser.add_argument("mode", nargs="?", default="reuse", choices=("reuse", "clean"))

    def _target(self) -> GateCommand:
        values = vars(self._args) | {
            "clean_build": self._args.mode == "clean" or self._args.clean_build,
            # The benchmark command owns these arguments; a focus run means
            # its complete default dimension set rather than invented knobs.
            "quick": False,
            "dimensions": "",
            "commit": "unknown",
        }
        return TARGETS[self._args.group](
            self._runner,
            argparse.Namespace(**values),
            qualification=self._qualification,
            invocation=self._invocation,
        )

    def plan(self) -> Plan:
        return self._target().plan()

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return self._target().resources(runner)

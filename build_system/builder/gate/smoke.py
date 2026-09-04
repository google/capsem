"""Focused developer feedback, and never release qualification.

`smoke` was seventy-nine lines that re-implemented the workspace setup, the
service teardown trap, and the pytest invocations that `_test-candidate-run`
had its own copies of. All three are components now, so this module is the part
that is actually about smoke: which groups can share a machine, and which two
cannot.

The two that cannot are the suspend/resume files. Under Apple VZ they are
host-resource sensitive -- an unrelated VM can make a resume fail before the
guest signals ready -- which is the same reason the full gate serializes them,
recorded once in `[execution.exclusives]` rather than twice in shell.
"""

from __future__ import annotations

from . import pytestsuite, vmproofs
from .actions import Run
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .lifecycle import Resource
from .plan import Plan
from .proc import Runner
from .service import Service
from .serviceschema import SmokeGroup
from .testmodules import InWorkspace
from .workspace import Workspace


def _group(config: GateConfig, group: SmokeGroup, *, serial: bool) -> Step:
    settings = config.smoke
    suite = pytestsuite.Suite(
        label=f"smoke.{group.name}",
        paths=group.paths,
        markers=group.markers,
        ignores=tuple(
            path
            for other in settings.serial_groups
            for path in other.paths
            if not serial and any(path.startswith(p) for p in group.paths)
        ),
        parallel=False,
        stop_at_first_failure=False,
        require_artifacts=False,
        contends=(config.exclusive("host_service"),) if serial else (),
    )
    argv = suite.argv(config)
    if group.parallel:
        argv += ["-n", str(group.parallel), "--dist=loadfile"]
    return step(
        suite.label,
        Run(argv, env={settings.run_id_variable: f"smoke-{group.name}"}),
        contends=suite.contends,
        kind=Kind.CAPSEM,
        needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
        speed=Speed.SLOW,
    )


class SmokeCommand(
    InWorkspace,
    GateCommand,
    name="smoke",
    help="focused developer integration feedback; never release qualification",
):
    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        # The workspace first, then the daemon inside it -- released in
        # reverse, so the service stops before its run directory goes, which
        # is what flushes `serial.log`. The service is constructed *from* the
        # workspace, so "which service" cannot drift from "which home".
        #
        # Built with the guarded runner, so the daemon it launches is recorded
        # and cannot itself start a second gate.
        workspace = Workspace(self._config)
        return (workspace, Service(self._config, workspace, runner))

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        base = config.suites.pytest.base_profile

        checked = plan.add(step("doctor", Run(config.smoke.doctor),
            kind=Kind.CAPSEM,
            needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
            speed=Speed.SLOW,
        ))
        injection = plan.add(vmproofs.injection(config, profile=base), after=(checked,))
        integration = plan.add(vmproofs.integration(config, profile=base), after=(injection,))

        parallel = [
            plan.add(_group(config, group, serial=False), after=(integration,))
            for group in config.smoke.groups
        ]
        previous = tuple(parallel)
        for group in config.smoke.serial_groups:
            previous = (plan.add(_group(config, group, serial=True), after=previous),)

        return plan

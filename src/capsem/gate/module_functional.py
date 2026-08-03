"""The module that runs the suites needing a booted VM, per profile."""

from __future__ import annotations

from . import (
    hostpackage,
    profiles,
    pytestsuite,
    vmproofs,
)
from .command import GateCommand
from .config import GateConfig
from .execution import Step
from .plan import Plan
from .qualification import Qualification
from .testmodules import InWorkspace


class FunctionalModule(
    InWorkspace,
    GateCommand,
    name="test-functional",
    help="every VM-owned suite, for every profile the channel selects",
):
    """The compatibility axis, and the slowest thing the gate does.

    The base profile takes the broad proof: everything that can share a
    machine, four VMs at a time. Each remaining profile then repeats the
    VM-owned suites -- that is the compatibility axis, not a reduced
    release-only substitute.

    What may not overlap is declared rather than achieved by placement. In
    shell these ran in sequence below a `wait` and stayed correct only because
    nobody added a job underneath.
    """

    def plan(self) -> Plan:
        plan = Plan(self.name)
        functional(plan, self._config, qualification=self.qualification)
        return plan


def functional(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
) -> Step:
    """Every VM-owned suite, for every profile the channel selects."""
    phase = plan.phase("functional")
    axis = profiles.selected(config)
    base, rest = axis[0], axis[1:]

    # A release lane was handed signed binaries; signing them again would
    # replace the bytes the manifest selected with locally built ones.
    first: tuple = after
    if not qualification.pulled:
        first = (phase.add(hostpackage.sign_step(config), after=after),)

    previous = _profile_lane(phase, config, base, after=first, broad=True)
    for profile in rest:
        previous = _profile_lane(phase, config, profile, after=(previous,), broad=False)
    return previous


def _profile_lane(phase, config: GateConfig, profile: str, *, after: tuple, broad: bool):
    """One profile's VM-owned suites, in the order they depend on.

    The base profile takes the broad proof -- everything that can share a
    machine, four VMs at a time. Each remaining profile repeats the VM-owned
    suites instead: that is the compatibility axis, not a reduced substitute.
    """
    head = (
        pytestsuite.broad(config, profile=profile)
        if broad
        else pytestsuite.compatibility(config, profile=profile)
    )
    current = phase.add(head.as_step(config), after=after)
    current = phase.add(
        pytestsuite.host_snapshot(config, profile=profile).as_step(config),
        after=(current,),
    )
    current = phase.add(
        pytestsuite.timing(config, profile=profile).as_step(config), after=(current,)
    )
    current = phase.add(vmproofs.injection(config, profile=profile), after=(current,))
    current = phase.add(vmproofs.integration(config, profile=profile), after=(current,))
    return phase.add(
        pytestsuite.benchmark(config, profile=profile).as_step(config), after=(current,)
    )

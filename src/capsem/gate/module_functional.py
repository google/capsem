"""The module that runs the suites needing a booted VM, per profile."""

from __future__ import annotations

from . import (
    hostpackage,
    profiles,
    pytestsuite,
    vmproofs,
)
from .actions import Action
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .execution import Step, step
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

    uses_qualification = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        functional(plan, self._config, qualification=self.qualification)
        return plan


class AxisAgrees(Action, name="axis-agrees"):
    """Check the materialized profiles are the ones the plan was built for.

    `selected()` reads checked-in `config/profiles/`; this reads what the
    build actually materialized, and refuses when they differ. A materialized
    catalog that does not match means the gate would prove a pairing nobody is
    shipping -- which is why the check did not go away when the plan stopped
    reading build output, it moved to where it can run.
    """

    def render(self) -> str:
        return "check the materialized profiles match the checked-in axis"

    def perform(self, context: Context) -> None:
        profiles.agree(context.config)
        context.journal.note(f"profile axis {', '.join(profiles.selected(context.config))}")


def functional(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
) -> Step:
    """Every VM-owned suite, for every profile the channel selects."""
    phase = plan.phase("functional")
    # From checked-in `config/profiles/`, because this runs while the plan is
    # being built and a plan may not depend on build output. See
    # `profiles.selected`.
    axis = profiles.selected(config)
    base, rest = axis[0], axis[1:]

    # That the materialized catalog agrees with the source axis and with the
    # manifest under test is still required -- it is simply a run-time
    # question now, asked once, before any profile lane runs against it.
    agreed = phase.add(step("axis", AxisAgrees()), after=after)

    # A release lane was handed signed binaries; signing them again would
    # replace the bytes the manifest selected with locally built ones.
    first: tuple = (agreed,)
    if not qualification.pulled:
        first = (phase.add(hostpackage.sign_step(config), after=(agreed,)),)

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

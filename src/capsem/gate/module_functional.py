"""The module that runs the suites needing a booted VM, per profile."""

from __future__ import annotations

from dataclasses import replace
from pathlib import Path

from . import (
    audits,
    hostpackage,
    profiles,
    pytestsuite,
    toolchain,
    vmproofs,
)
from .actions import Action
from .command import GateCommand
from .config import GateConfig
from .content import ProfileContent
from .context import Context
from .execution import Kind, Needs, Speed, Step, step
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

    def __init__(self, assets: Path | None = None, profiles_dir: Path | None = None) -> None:
        self._assets = assets
        self._profiles_dir = profiles_dir

    def render(self) -> str:
        return "check the materialized profiles match the checked-in axis"

    def perform(self, context: Context) -> None:
        profiles.agree(
            context.config,
            profiles_dir=self._profiles_dir,
            manifest=(
                self._assets / context.config.install.manifest_name
                if self._assets is not None
                else None
            ),
        )
        context.journal.note(f"profile axis {', '.join(profiles.selected(context.config))}")


def functional(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
    isolated_assets: bool = False,
    staged: ProfileContent | None = None,
    generated: Step | None = None,
) -> Step:
    """Every VM-owned suite, for every profile the channel selects.

    `staged` is absolute, and only a release lane passes it. That lane stages
    its cohort into the workspace and then qualifies from a private prefix
    which carries none of it, so a checkout-relative answer points at a
    directory nothing ever wrote.
    """
    phase = plan.phase("functional")
    # From checked-in `config/profiles/`, because this runs while the plan is
    # being built and a plan may not depend on build output. See
    # `profiles.selected`.
    axis = profiles.selected(config)
    base, rest = axis[0], axis[1:]

    # That the materialized catalog agrees with the source axis and with the
    # manifest under test is still required -- it is simply a run-time
    # question now, asked once, before any profile lane runs against it.
    base_content = _profile_content(config, base) if isolated_assets else None
    agreed = phase.add(
        step(
            "axis",
            AxisAgrees(
                assets=staged.assets if staged else (base_content[0] if base_content else None),
                profiles_dir=(
                    staged.profiles(config)
                    if staged
                    else (base_content[1] if base_content else None)
                ),
            ),
            kind=Kind.UNIT_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=after,
    )

    # This module owns its prerequisites, the same way `module_contracts` had
    # to learn to. The broad suite renders the release site from fixtures with
    # `pnpm --dir release-site run build`, and `node_modules` is gitignored --
    # so a local run worked on whatever an earlier phase had installed, and the
    # release lane, whose prefix carries only tracked files, died on a missing
    # Astro. Idempotent, and the `node_modules` exclusive keeps the two
    # installs in a candidate plan from overlapping.
    ready = phase.add(
        toolchain.node(config, config.functional.node_workspaces), after=(agreed,)
    )
    # The third module to need this, for the reason its own docstring gives:
    # the generated mock is gitignored, so it is never part of the source a run
    # is given, and the broad suite checks it for staleness. In the fast lane
    # this rides along with work already being done. Here it is real added
    # cost -- an `mcp_export` build in a lane that otherwise compiles no Rust --
    # and the alternative is a suite that can only pass on a warm checkout.
    # As in `static`: made here when this module runs alone, and handed over
    # when a composed run has already made it.
    #
    # `ready` stays in the chain either way. The handed-over step lives in an
    # earlier phase, so depending on it *instead* dropped this module's own
    # ordering: its suites became reachable before the artifacts they boot, and
    # the phase-order contract caught the plan with `functional` at 29 and
    # `artifacts` at 95.
    settled: tuple[Step, ...] = (
        (generated, ready)
        if generated is not None
        else (phase.add(audits.generated_settings(config), after=(ready,)),)
    )

    # A release lane was handed signed binaries; signing them again would
    # replace the bytes the manifest selected with locally built ones.
    first: tuple = settled
    if not qualification.pulled:
        first = (phase.add(hostpackage.sign_step(config), after=settled),)

    previous = _profile_lane(
        phase,
        config,
        base,
        after=first,
        broad=True,
        isolated_assets=isolated_assets,
        staged=staged,
    )
    for profile in rest:
        previous = _profile_lane(
            phase,
            config,
            profile,
            after=(previous,),
            broad=False,
            isolated_assets=isolated_assets,
            staged=staged,
        )
    return previous


def _profile_content(config: GateConfig, profile: str) -> tuple[Path, Path]:
    root = config.path(config.assets.test_root) / profile
    return (
        root / config.assets.merged_assets_dir,
        root / config.assets.merged_config_dir / config.assets.materialized_profiles_dir,
    )


def _profile_lane(
    phase,
    config: GateConfig,
    profile: str,
    *,
    after: tuple,
    broad: bool,
    isolated_assets: bool,
    staged: ProfileContent | None = None,
):
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
    # A release lane's cohort is one staged pair for every profile, not a
    # private tree per profile. Without this the suites inherit no content
    # selection at all and fall back to the checkout -- which, inside the
    # prefix, is the one place the lane never staged anything.
    if staged is not None:
        assets, profiles_dir = staged.assets, staged.profiles(config)
    elif isolated_assets:
        assets, profiles_dir = _profile_content(config, profile)
    else:
        assets, profiles_dir = None, None

    def selected(suite):
        if assets is None or profiles_dir is None:
            return suite
        return replace(suite, assets_dir=str(assets), profiles_dir=str(profiles_dir))

    head = selected(head)
    current = phase.add(head.as_step(config), after=after)
    current = phase.add(
        selected(pytestsuite.host_snapshot(config, profile=profile)).as_step(config),
        after=(current,),
    )
    current = phase.add(
        selected(pytestsuite.timing(config, profile=profile)).as_step(config), after=(current,)
    )
    current = phase.add(
        vmproofs.injection(
            config,
            profile=profile,
            assets=str(assets) if assets else None,
            profiles_dir=str(profiles_dir) if profiles_dir else None,
        ),
        after=(current,),
    )
    current = phase.add(
        vmproofs.integration(
            config,
            profile=profile,
            assets=str(assets) if assets else None,
            profiles_dir=str(profiles_dir) if profiles_dir else None,
        ),
        after=(current,),
    )
    return phase.add(
        selected(pytestsuite.benchmark(config, profile=profile)).as_step(config),
        after=(current,),
    )

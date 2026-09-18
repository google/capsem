"""The module that runs the suites needing a booted VM, per profile."""

from __future__ import annotations

from dataclasses import dataclass, replace
from pathlib import Path

from . import (
    audits,
    hostpackage,
    kingslanding,
    profiles,
    pytestsuite,
    runtimeprepare,
    sdkchecks,
    toolchain,
    vmproofs,
)
from .command import GateCommand
from .config import GateConfig
from .content import ProfileContent
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan
from .profileaxis import AxisAgrees
from .qualification import Qualification
from .testmodules import InWorkspace


class FunctionalModule(
    InWorkspace,
    GateCommand,
    name="test-functional",
    help="every VM-owned suite, for every profile the channel selects",
):
    """The compatibility axis, and the slowest thing the gate does.

    The base profile takes the broad proof. Each remaining profile repeats
    every VM suite, including Kingslanding, as the compatibility axis.

    What may not overlap is declared rather than achieved by placement. In
    shell these ran in sequence below a `wait` and stayed correct only because
    nobody added a job underneath.
    """

    uses_qualification = True
    outside_egress = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        if self.qualification.pulled:
            functional(plan, self._config, qualification=self.qualification)
        else:
            permission = self.rebuild_permission
            prepared = runtimeprepare.prepare(plan, self._config, permission=permission)
            functional(
                plan,
                self._config,
                qualification=self.qualification,
                after=(prepared.ready,),
                signed=prepared.ready,
            )
        return plan


def functional(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
    isolated_assets: bool = False,
    staged: ProfileContent | None = None,
    generated: Step | None = None,
    node: Step | None = None,
    phase_name: str = "functional",
    axis: tuple[str, ...] | None = None,
    benchmark: bool = True,
    signed: Step | None = None,
    source_contracts_proved: bool = False,
) -> Step:
    """Every VM-owned suite, for every profile the channel selects.

    `staged` is absolute, and only a release lane passes it. That lane stages
    its cohort into the workspace and then qualifies from a private prefix
    which carries none of it, so a checkout-relative answer points at a
    directory nothing ever wrote.

    `phase_name` and `axis` are how the local rehearsal replays this phase
    against a pulled cohort without colliding with the candidate's own
    `functional` steps, for the reason `pulled_artifacts` already documents:
    two steps cannot share a label.

    `benchmark` is off for that rehearsal, and it is the one suite here that
    must not run twice. The others answer a question about the pulled cohort --
    does this content resolve, do these binaries exist, does a VM boot from
    them -- and the answer differs from the candidate's. The benchmark records
    a performance baseline, and a second recording of the same profile on the
    same machine in the same run is not a second measurement of anything: it is
    the first one repeated on a warm host, and it leaves the profile with two
    baselines where the contract says one. Four of the eight binary-release failures
    were here rather than in the five steps the rehearsal used to cover, and
    every one of them died within four seconds on a precondition -- a missing
    profiles directory, initrd, generated file or host binary. Those are
    minutes of local work that were being paid for at dispatch prices.
    """
    phase = plan.phase(phase_name)
    # From checked-in `config/profiles/`, because this runs while the plan is
    # being built and a plan may not depend on build output. See
    # `profiles.selected`. A caller may narrow it: the rehearsal proves the
    # pulled path, which is the same for every profile, and the compatibility
    # axis is what the candidate's own `functional` phase is for.
    proven = tuple(profiles.selected(config)) if axis is None else axis
    base = proven[0]

    # That the materialized catalog agrees with the source axis and the manifest
    # under test is still required: a run-time question, asked once, before any lane.
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
    # `pnpm --dir build_system/release_site run build`, and `node_modules` is gitignored --
    # so a local run worked on whatever an earlier phase had installed, and the
    # release lane, whose prefix carries only tracked files, died on a missing
    # Astro. Idempotent, and the `node_modules` exclusive keeps the two
    # installs in a candidate plan from overlapping.
    #
    # Handed over when a composed run already installed it, exactly as
    # `generated` is below. Not `plan.shared`: two lanes want this step at two
    # different points in the order, so a single shared node inherits both
    # sets of edges and closes a cycle -- which is the reordering
    # `_already_issuing` documents as the reason dedup lives at the call site
    # rather than inside `Plan.add`.
    prepared: tuple[Step, ...] = (
        (node, agreed)
        if node is not None
        else (
            phase.add(toolchain.node(config, config.functional.node_workspaces), after=(agreed,)),
        )
    )
    prepared = (*prepared, *sdkchecks.braavos(plan, phase, config, after=prepared))
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
        (generated, *prepared)
        if generated is not None
        else (phase.add(audits.generated_settings(config), after=prepared),)
    )

    # A release lane was handed signed binaries; signing them again would
    # replace the bytes the manifest selected with locally built ones.
    first: tuple = settled
    if not qualification.pulled:
        first = (
            (signed, *settled)
            if signed is not None
            else (phase.add(hostpackage.sign_step(config), after=settled),)
        )

    fixture = phase.add(kingslanding.prefetch(config), after=first)
    # Each profile's VM suites share the machine with the other profile's: they
    # claim Apple VZ and the workspace binaries shared, and the two xdist
    # suites claim the VM fleet alone so their eight VMs never overlap. Order
    # within a lane is kept, bounding the machine to one suite per profile.
    lanes = tuple(
        _profile_lane(
            phase,
            config,
            profile,
            after=(fixture,),
            broad=profile == base,
            content=_content_selector(config, profile, staged=staged, isolated=isolated_assets),
            source_contracts_proved=source_contracts_proved and profile == base,
        )
        for profile in proven
    )
    # Measurements hold Apple VZ alone, so they wait for every lane and then
    # run one at a time; the declared order gives the phase one end.
    current: tuple[Step, ...] = lanes
    for profile in proven:
        content = _content_selector(config, profile, staged=staged, isolated=isolated_assets)
        measured = [pytestsuite.timing(config, profile=profile)]
        if benchmark:
            measured += [
                kingslanding.benchmark_suite(config, profile=profile),
                pytestsuite.benchmark(config, profile=profile),
            ]
        for suite in measured:
            current = (phase.add(content.suite(suite).as_step(config), after=current),)
    return current[0]


@dataclass(frozen=True)
class _Content:
    """The content one profile's suites and VM proofs are pointed at."""

    assets: Path | None
    profiles_dir: Path | None

    def suite(self, suite: pytestsuite.Suite) -> pytestsuite.Suite:
        if self.assets is None or self.profiles_dir is None:
            return suite
        return replace(suite, assets_dir=str(self.assets), profiles_dir=str(self.profiles_dir))

    def proof_arguments(self) -> dict[str, str | None]:
        return {
            "assets": str(self.assets) if self.assets else None,
            "profiles_dir": str(self.profiles_dir) if self.profiles_dir else None,
        }


def _content_selector(
    config: GateConfig, profile: str, *, staged: ProfileContent | None, isolated: bool
) -> _Content:
    # A release lane's cohort is one staged pair for every profile, not a
    # private tree per profile. Without this the suites inherit no content
    # selection at all and fall back to the checkout -- which, inside the
    # prefix, is the one place the lane never staged anything.
    if staged is not None:
        return _Content(staged.assets, staged.profiles(config))
    if isolated:
        return _Content(*_profile_content(config, profile))
    return _Content(None, None)


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
    content: _Content,
    source_contracts_proved: bool = False,
) -> Step:
    """One profile's VM-owned suites that can share the machine, in order.

    The base profile takes the broad proof -- everything that can share a
    machine, four VMs at a time. Each remaining profile repeats the VM-owned
    suites instead: that is the compatibility axis, not a reduced substitute.
    Timing and benchmarks are not here; they need the machine to themselves.
    """
    head = (
        pytestsuite.broad(config, profile=profile, source_contracts_proved=source_contracts_proved)
        if broad
        else pytestsuite.compatibility(config, profile=profile)
    )
    current = phase.add(content.suite(head).as_step(config), after=after)
    for owned in (
        kingslanding.suite(config, profile=profile, benchmark=False),
        kingslanding.greyjoy_suite(config, profile=profile),
        pytestsuite.host_snapshot(config, profile=profile),
    ):
        current = phase.add(content.suite(owned).as_step(config), after=(current,))
    current = phase.add(
        vmproofs.injection(config, profile=profile, **content.proof_arguments()), after=(current,)
    )
    return phase.add(
        vmproofs.integration(config, profile=profile, **content.proof_arguments()),
        after=(current,),
    )

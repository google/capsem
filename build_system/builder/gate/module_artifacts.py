"""The module that proves the built artifacts before anything boots them.

One of three release phases that shared a file. The split is mechanical: no
plan line and no edge changes, which is what its guard asserts.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from . import (
    assetplan,
    pytestsuite,
    toolchain,
    webaudits,
)
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan
from .qualification import BinaryQualification, ProfileQualification, Qualification
from .testmodules import InWorkspace


class ArtifactsModule(
    InWorkspace,
    GateCommand,
    name="test-artifacts",
    help="build every profile's VM assets, or verify the pulled ones",
):
    """Two shapes, one module.

    A local run builds every profile for both architectures and boots each
    one. A release lane arrives with immutable artifacts already resolved from
    a manifest and verifies exactly those instead -- rebuilding them would
    prove something about the source rather than about what ships.
    """

    uses_qualification = True
    outside_egress = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        artifacts(plan, self._config, qualification=self.qualification)
        return plan


class ProfileArtifactsModule(
    InWorkspace,
    GateCommand,
    name="test-profile-artifacts",
    help="verify and boot one staged profile without claiming a binary pairing",
):
    """The non-activation half of a cold-channel profile release.

    A first profile can be published immutably before the channel has a package
    cohort.  That is not a fourth release qualification: it cannot run the
    functional or glow-up modules.  It is exactly the profile-owned artifact
    proof, exposed as its own private command so the workflow never invents a
    package merely to satisfy the complete pairing type.
    """

    @classmethod
    def add_arguments(cls, parser: argparse.ArgumentParser) -> None:
        parser.add_argument("input_dir", type=Path)
        parser.add_argument("profile")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        pulled_artifacts(
            plan,
            self._config,
            input_dir=self._args.input_dir,
            profile=self._args.profile,
        )
        return plan


def artifacts(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
    node: Step | None = None,
    bundled: Step | None = None,
) -> Step:
    """Build every profile's VM assets, or verify the pulled ones."""
    phase = plan.phase("artifacts")
    settings = config.modules

    if isinstance(qualification, (BinaryQualification, ProfileQualification)):
        return pulled_artifacts(
            plan,
            config,
            input_dir=qualification.input_dir,
            profile=qualification.profile,
            after=after,
        )

    built = assetplan.fragment(plan, config, after=after)
    installed = node or phase.add(
        toolchain.node(config, (config.frontend.workspace,)), after=after
    )
    prerequisites = (*after, installed) if node is not None else (installed,)
    frontend = bundled or phase.add(webaudits.frontend_bundle(config), after=prerequisites)
    return phase.add(
        pytestsuite.Suite(
            label="build-chain",
            paths=settings.build_chain_artifact_tests,
            stop_at_first_failure=False,
            # `test_cargo_build.py` builds the workspace. Wearing a pytest
            # label makes that no less true, and the target directory is the
            # same one every other build locks.
            contends=(config.exclusive("workspace_binaries"),),
        ).as_step(config),
        after=(built, frontend),
    )


def pulled_artifacts(
    plan: Plan,
    config: GateConfig,
    *,
    input_dir: str | Path,
    profile: str | None,
    after: tuple[Step, ...] = (),
    phase_name: str = "artifacts",
) -> Step:
    """Verify pulled inputs, and boot the one profile when one is selected.

    `phase_name` is how the local rehearsal keeps its copy of this step apart
    from the candidate's own `artifacts` phase, which in that plan is the
    build. Two steps cannot share a label, and a rehearsal that had to be
    renamed by hand would be a rehearsal of something else.
    """
    phase = plan.phase(phase_name)
    settings = config.modules
    verify = phase.add(
        step(
            "release-inputs.verify",
            Script(config, settings.verify_inputs_script, "--input-dir", input_dir),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=after,
    )
    # A binary lane resolves every profile the manifest names, so there is no
    # single one to boot; profile releases and deferred staging select one.
    if profile is None:
        return verify
    return phase.add(
        step(
            "release-inputs.boot",
            Script(
                config,
                settings.prove_profile_assets_script,
                "--input-dir",
                input_dir,
                "--profile",
                profile,
            ),
            contends=(config.exclusive("apple_vz"),),
            kind=Kind.CAPSEM,
            needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(verify,),
    )

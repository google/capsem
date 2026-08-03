"""The module that proves the built artifacts before anything boots them.

One of three release phases that shared a file. The split is mechanical: no
plan line and no edge changes, which is what its guard asserts.
"""

from __future__ import annotations

from . import (
    assetplan,
    pytestsuite,
)
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Step, step
from .plan import Plan
from .qualification import Qualification
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

    def plan(self) -> Plan:
        plan = Plan(self.name)
        artifacts(plan, self._config, qualification=self.qualification)
        return plan


def artifacts(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
) -> Step:
    """Build every profile's VM assets, or verify the pulled ones."""
    phase = plan.phase("artifacts")
    settings = config.modules

    if qualification.pulled:
        verify = phase.add(
            step(
                "release-inputs.verify",
                Script(settings.verify_inputs_script, "--input-dir", qualification.input_dir),
            ),
            after=after,
        )
        # A binary lane resolves every profile the manifest names, so there is
        # no single one to boot; a profile lane is publishing exactly one.
        if qualification.profile is None:
            return verify
        return phase.add(
            step(
                "release-inputs.boot",
                Script(
                    settings.prove_profile_assets_script,
                    "--input-dir",
                    qualification.input_dir,
                    "--profile",
                    qualification.profile,
                ),
                contends=(config.exclusive("apple_vz"),),
            ),
            after=(verify,),
        )

    built = assetplan.fragment(plan, config, after=after)
    return phase.add(
        pytestsuite.Suite(
            label="build-chain",
            paths=settings.build_chain_artifact_tests,
            stop_at_first_failure=False,
        ).as_step(config),
        after=(built,),
    )

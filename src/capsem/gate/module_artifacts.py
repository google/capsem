"""The module that proves the built artifacts before anything boots them.

One of three release phases that shared a file. The split is mechanical: no
plan line and no edge changes, which is what its guard asserts.
"""

from __future__ import annotations

import os

from . import (
    assets as assetgate,
)
from . import (
    pytestsuite,
)
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Step, step
from .plan import Plan
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

    def plan(self) -> Plan:
        plan = Plan(self.name)
        artifacts(plan, self._config)
        return plan


def artifacts(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Build every profile's VM assets, or verify the pulled ones."""
    phase = plan.phase("artifacts")
    settings = config.modules
    pulled = os.environ.get(settings.release_input_dir)

    if pulled:
        verify = phase.add(
            step(
                "release-inputs.verify",
                Script(settings.verify_inputs_script, "--input-dir", pulled),
            ),
            after=after,
        )
        profile = os.environ.get(settings.release_profile)
        if not profile:
            return verify
        return phase.add(
            step(
                "release-inputs.boot",
                Script(
                    settings.prove_profile_assets_script,
                    "--input-dir",
                    pulled,
                    "--profile",
                    profile,
                ),
                contends=(config.exclusive("apple_vz"),),
            ),
            after=(verify,),
        )

    built = assetgate.fragment(plan, config, after=after)
    return phase.add(
        pytestsuite.Suite(
            label="build-chain",
            paths=settings.build_chain_artifact_tests,
            stop_at_first_failure=False,
        ).as_step(config),
        after=(built,),
    )

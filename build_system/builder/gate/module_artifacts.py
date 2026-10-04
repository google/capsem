"""The module that proves the built artifacts before anything boots them.

One of three release phases that shared a file. The split is mechanical: no
plan line and no edge changes, which is what its guard asserts.
"""

from __future__ import annotations

from pathlib import Path

from . import (
    assetplan,
    pytestsuite,
    sdkchecks,
    toolchain,
    webaudits,
)
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan
from .qualification import Qualification
from .testmodules import InWorkspace


class ArtifactsModule(
    InWorkspace,
    GateCommand,
    name="test-artifacts",
    help="build the VM runtime assets, or verify the pulled ones",
):
    """Two shapes, one module.

    A local run builds the runtime for both architectures and boots it. A
    release lane arrives with immutable artifacts already resolved from
    a manifest and verifies exactly those instead -- rebuilding them would
    prove something about the source rather than about what ships.
    """

    uses_qualification = True
    outside_egress = True

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
    node: Step | None = None,
    bundled: Step | None = None,
) -> Step:
    """Build the VM runtime assets, or verify the pulled ones."""
    phase = plan.phase("artifacts")
    settings = config.modules

    if qualification.input_dir is not None:
        return pulled_artifacts(
            plan,
            config,
            input_dir=qualification.input_dir,
            boot=qualification.runtime,
            after=after,
        )

    built = assetplan.fragment(plan, config, after=after)
    installed = node or phase.add(
        toolchain.node(config, (config.frontend.workspace,)), after=after
    )
    prerequisites = (*after, installed) if node is not None else (installed,)
    if bundled is None:
        sdk = phase.add(sdkchecks.typescript_bundle(config), after=prerequisites)
        bundled = phase.add(webaudits.frontend_bundle(config), after=(sdk,))
    frontend = bundled
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
    boot: bool,
    after: tuple[Step, ...] = (),
    phase_name: str = "artifacts",
) -> Step:
    """Verify pulled inputs, and boot the runtime when it is the candidate.

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
    # A binary lane boots the pulled runtime through its functional phase; a
    # runtime release boots it here first, alone, before any pairing.
    if not boot:
        return verify
    return phase.add(
        step(
            "release-inputs.boot",
            Script(
                config,
                settings.prove_runtime_assets_script,
                "--input-dir",
                input_dir,
            ),
            contends=(config.exclusive("apple_vz"),),
            kind=Kind.CAPSEM,
            needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(verify,),
    )

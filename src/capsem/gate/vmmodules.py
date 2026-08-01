"""The modules that need built artifacts, a VM, or both.

Split from `testmodules`, which holds the ones provable from a bare checkout.
The seam is what a module needs before it can start: these three cannot run
against source alone, and the others cannot tell you anything source does not
already contain.
"""

from __future__ import annotations

import os

from . import (
    assets as assetgate,
)
from . import (
    crosscompile,
    host,
    hostpackage,
    install,
    profiles,
    pytestsuite,
    vmproofs,
)
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Step, step
from .plan import Plan
from .testmodules import InWorkspace, storagerelease


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
                    "--input-dir", pulled,
                    "--profile", profile,
                ),
                contends=(config.exclusive("apple_vz"),),
            ),
            after=(verify,),
        )

    built = phase.add(assetgate.assets_step(config), after=after)
    return phase.add(
        pytestsuite.Suite(
            label="build-chain",
            paths=settings.build_chain_artifact_tests,
            stop_at_first_failure=False,
        ).as_step(config),
        after=(built,),
    )


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
        functional(plan, self._config)
        return plan


def functional(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Every VM-owned suite, for every profile the channel selects."""
    phase = plan.phase("functional")
    axis = profiles.selected(config)
    base, rest = axis[0], axis[1:]

    first: tuple = after
    if not os.environ.get(config.modules.release_input_dir):
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


class GlowupModule(
    InWorkspace,
    GateCommand,
    name="test-glowup",
    help="build the release packages and prove an install upgrades cleanly",
):
    """The end of the gate, and the only part that installs anything.

    Two shapes again. A release lane arrives with the publishable package
    already built and proves the glow-up against it, twice: once as staged,
    and once with the release environment cleared so the channel switch has to
    rediscover its state from the installed system rather than inherit what it
    was told.

    A local run builds both architectures first, and on macOS also builds,
    signs and installs the real `.pkg` inside a disposable Tart VM -- the one
    proof a hosted macOS runner cannot make, because it cannot nest Apple
    Virtualization.framework.
    """

    def plan(self) -> Plan:
        plan = Plan(self.name)
        glowup(plan, self._config)
        return plan


def glowup(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Build the release packages and prove an install upgrades cleanly."""
    phase = plan.phase("glowup")
    package = os.environ.get(config.modules.release_package)
    if package:
        return _prove_pulled_package(phase, config, package, after)
    return _build_and_prove(plan, phase, config, after)


# -- the release lane ------------------------------------------------------


def _prove_pulled_package(phase, config: GateConfig, package: str, after: tuple) -> Step:
    """The publishable package, proved twice.

    Once as staged, and once with the release environment cleared, so the
    channel switch has to rediscover its state from the installed system
    rather than inherit what it was told.
    """
    settings = config.modules
    staged = phase.add(
        _glowup_step(config, "package", package, settings.glowup_work_dir), after=after
    )
    return phase.add(
        _glowup_step(
            config,
            "channel-switch",
            package,
            settings.channel_switch_work_dir,
            clear=settings.channel_switch_cleared,
        ),
        after=(staged,),
    )


def _glowup_step(
    config: GateConfig, label: str, package: str, work_dir: str, *, clear: tuple = ()
) -> Step:
    settings = config.modules
    functional_settings = config.functional
    return step(
        label,
        Script(
            settings.glowup_script,
            "--input-deb", package,
            "--bin-dir", os.environ.get(settings.release_bin_dir, settings.default_bin_dir),
            "--assets-dir",
            os.environ.get(functional_settings.assets_variable, functional_settings.assets_dir),
            "--config-root",
            os.environ.get(
                functional_settings.config_root_variable, functional_settings.config_root
            ),
            "--work-dir", work_dir,
            "--package-ready",
            env=dict.fromkeys(clear, ""),
        ),
        contends=(config.exclusive("docker_daemon"),),
    )


# -- the local lane --------------------------------------------------------


def _build_and_prove(plan: Plan, phase, config: GateConfig, after: tuple) -> Step:
    # `previous` chains each architecture behind the last; the first has
    # nothing before it beyond whatever this phase was given.
    previous: tuple = after
    last = list(config.architectures)[-1]
    for arch in config.architectures:
        built = crosscompile.fragment(plan, config, config.arch(arch), after=previous)
        released = phase.add(
            storagerelease(config, f"completed-package-{arch}"), after=(built,)
        )
        # Between the two package builds, not after both: the second build
        # needs the headroom the install rail is still reserving.
        previous = (
            (phase.add(storagerelease(config, "deferred-install-target"), after=(released,)),)
            if arch != last
            else (released,)
        )

    # Nothing releases capsem-host-builder here. Both package builds need it,
    # and so does `docker/Dockerfile.install-test`, which the install proof
    # always rebuilds from -- so `after-install` is the earliest boundary at
    # which nothing derives from that tag any more.
    if host.on_macos():
        previous = (
            phase.add(
                step(
                    "macos-package",
                    Script(config.modules.macos_glowup_script),
                    contends=(config.exclusive("apple_vz"),),
                ),
                after=previous,
            ),
        )

    sbom = phase.add(hostpackage.sbom_step(config), after=previous)
    return phase.add(install.install_step(config), after=(sbom,))

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
from .execution import step
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
        config = self._config
        settings = config.modules
        pulled = os.environ.get(settings.release_input_dir)

        if pulled:
            verify = plan.add(
                step(
                    "release-inputs.verify",
                    Script(settings.verify_inputs_script, "--input-dir", pulled),
                )
            )
            profile = os.environ.get(settings.release_profile)
            if profile:
                plan.add(
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
            return plan

        assets = plan.add(assetgate.assets_step(config))
        plan.add(
            pytestsuite.Suite(
                label="build-chain",
                paths=settings.build_chain_artifact_tests,
                stop_at_first_failure=False,
            ).as_step(config),
            after=(assets,),
        )
        return plan


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
        config = self._config
        axis = profiles.selected(config)
        base, rest = axis[0], axis[1:]

        first: tuple = ()
        if not os.environ.get(config.modules.release_input_dir):
            first = (plan.add(hostpackage.sign_step(config)),)

        broad = plan.add(
            pytestsuite.broad(config, profile=base).as_step(config), after=first
        )
        snapshot = plan.add(
            pytestsuite.host_snapshot(config, profile=base).as_step(config),
            after=(broad,),
        )
        timing = plan.add(
            pytestsuite.timing(config, profile=base).as_step(config), after=(snapshot,)
        )

        injection = plan.add(vmproofs.injection(config, profile=base), after=(timing,))
        integration = plan.add(
            vmproofs.integration(config, profile=base), after=(injection,)
        )
        previous = plan.add(
            pytestsuite.benchmark(config, profile=base).as_step(config),
            after=(integration,),
        )

        for profile in rest:
            compatibility = plan.add(
                pytestsuite.compatibility(config, profile=profile).as_step(config),
                after=(previous,),
            )
            snapshot = plan.add(
                pytestsuite.host_snapshot(config, profile=profile).as_step(config),
                after=(compatibility,),
            )
            timing = plan.add(
                pytestsuite.timing(config, profile=profile).as_step(config),
                after=(snapshot,),
            )
            injection = plan.add(
                vmproofs.injection(config, profile=profile), after=(timing,)
            )
            integration = plan.add(
                vmproofs.integration(config, profile=profile), after=(injection,)
            )
            previous = plan.add(
                pytestsuite.benchmark(config, profile=profile).as_step(config),
                after=(integration,),
            )

        return plan


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
        config = self._config
        settings = config.modules
        package = os.environ.get(settings.release_package)

        if package:
            return self._prove_pulled_package(plan, package)
        return self._build_and_prove(plan)

    # -- the release lane --------------------------------------------------

    def _prove_pulled_package(self, plan: Plan, package: str) -> Plan:
        settings = self._config.modules
        staged = plan.add(self._glowup("glowup.package", package, settings.glowup_work_dir))
        plan.add(
            self._glowup(
                "glowup.channel-switch",
                package,
                settings.channel_switch_work_dir,
                # Cleared rather than overridden: the switch has to rediscover
                # the channel from installed state, and inheriting the previous
                # run's answer would prove nothing about that.
                clear=settings.channel_switch_cleared,
            ),
            after=(staged,),
        )
        return plan

    def _glowup(self, label: str, package: str, work_dir: str, *, clear: tuple = ()):
        config = self._config
        settings = config.modules
        functional = config.functional
        return step(
            label,
            Script(
                settings.glowup_script,
                "--input-deb", package,
                "--bin-dir", os.environ.get(settings.release_bin_dir, settings.default_bin_dir),
                "--assets-dir", os.environ.get(functional.assets_variable, functional.assets_dir),
                "--config-root", os.environ.get(functional.config_root_variable, functional.config_root),
                "--work-dir", work_dir,
                "--package-ready",
                env=dict.fromkeys(clear, ""),
            ),
            contends=(config.exclusive("docker_daemon"),),
        )

    # -- the local lane ----------------------------------------------------

    def _build_and_prove(self, plan: Plan) -> Plan:
        config = self._config

        # `previous` chains each architecture behind the last; the first has
        # nothing before it, which is what the empty tuple means.
        previous: tuple = ()
        last = list(config.architectures)[-1]
        for arch in config.architectures:
            built = crosscompile.fragment(
                plan, config, config.arch(arch), after=previous
            )
            released = plan.add(
                storagerelease(config, f"completed-package-{arch}"), after=(built,)
            )
            # Between the two package builds, not after both: the second build
            # needs the headroom the install rail is still reserving.
            previous = (
                (plan.add(storagerelease(config, "deferred-install-target"), after=(released,)),)
                if arch != last
                else (released,)
            )

        # capsem-host-builder is a dependency of both package builds, so its
        # final tag is released only after the second consumer -- never between
        # the assets and the package assembly.
        graph = plan.add(storagerelease(config, "completed-buildkit-graph"), after=previous)

        if host.on_macos():
            graph = plan.add(
                step(
                    "macos-package",
                    Script(config.modules.macos_glowup_script),
                    contends=(config.exclusive("apple_vz"),),
                ),
                after=(graph,),
            )

        sbom = plan.add(hostpackage.sbom_step(config), after=(graph,))
        plan.add(install.install_step(config), after=(sbom,))
        return plan

"""The module that packages, installs, and proves an upgrade transition.

Both lanes live here: the release lane proves a package pulled by digest, and
the local lane builds one per architecture and proves that.
"""

from __future__ import annotations

import os

from . import (
    crosscompile,
    host,
    hostpackage,
    install,
)
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Step, step
from .plan import Plan
from .testmodules import InWorkspace, storagerelease


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
            "--input-deb",
            package,
            "--bin-dir",
            os.environ.get(settings.release_bin_dir, settings.default_bin_dir),
            "--assets-dir",
            os.environ.get(functional_settings.assets_variable, functional_settings.assets_dir),
            "--config-root",
            os.environ.get(
                functional_settings.config_root_variable, functional_settings.config_root
            ),
            "--work-dir",
            work_dir,
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
        released = phase.add(storagerelease(config, f"completed-package-{arch}"), after=(built,))
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

"""The module that packages, installs, and proves an upgrade transition.

Both lanes live here: the release lane proves a package pulled by digest, and
the local lane builds one per architecture and proves that.
"""

from __future__ import annotations

from . import (
    crosscompile,
    host,
    hostpackage,
    install,
)
from .actions import Call, Script
from .command import GateCommand
from .config import GateConfig
from .content import LocalInstallContent, ProfileContent
from .execution import Step, step
from .opacity import CallJustification, OpaqueKind, machine_effects
from .plan import Plan
from .qualification import Qualification
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

    uses_qualification = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        glowup(plan, self._config, qualification=self.qualification)
        return plan


def glowup(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
) -> Step:
    """Build the release packages and prove an install upgrades cleanly."""
    phase = plan.phase("glowup")
    if qualification.pulled:
        return _prove_pulled_package(phase, config, qualification, after)
    return _build_and_prove(plan, phase, config, after)


# -- the release lane ------------------------------------------------------


def _prove_pulled_package(
    phase, config: GateConfig, qualification: Qualification, after: tuple
) -> Step:
    """The publishable package, proved twice.

    Once as staged, and once with the release environment cleared, so the
    channel switch has to rediscover its state from the installed system
    rather than inherit what it was told.
    """
    settings = config.modules
    content = ProfileContent.standalone(config)
    verified = phase.add(
        step(
            "content",
            Call(
                "verify one paired manifest-selected content bundle",
                lambda _context: content.require_complete(
                    config,
                    arches=(config.host_arch(),),
                ),
                justification=CallJustification(
                    kind=OpaqueKind.PURE_INSPECTION,
                    reason="release glow-up consumes one inseparable assets/config cohort",
                    effects=machine_effects(),
                ),
            ),
        ),
        after=after,
    )
    staged = phase.add(
        _glowup_step(
            config,
            "package",
            qualification,
            settings.glowup_work_dir,
            content,
        ),
        after=(verified,),
    )
    return phase.add(
        _glowup_step(
            config,
            "channel-switch",
            qualification,
            settings.channel_switch_work_dir,
            content,
            clear=settings.channel_switch_cleared,
        ),
        after=(staged,),
    )


def _glowup_step(
    config: GateConfig,
    label: str,
    qualification: Qualification,
    work_dir: str,
    content: ProfileContent,
    *,
    clear: tuple = (),
) -> Step:
    settings = config.modules
    return step(
        label,
        Script(
            settings.glowup_script,
            "--input-deb",
            qualification.package,
            "--bin-dir",
            qualification.bin_dir,
            "--assets-dir",
            content.assets,
            "--config-root",
            content.config,
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
    content = ProfileContent.isolated(
        config,
        config.path(config.assets.test_root) / config.suites.pytest.base_profile,
    )
    for arch in config.architectures:
        # The final install step below authors a checked local release graph
        # before installing the exact native package and running the broader
        # install/glow-up proof. Letting the narrower package phase hydrate
        # from mutable public stable first makes a broken channel impossible
        # to recover through either supported release command.
        built = crosscompile.fragment(
            plan,
            config,
            config.arch(arch),
            content=content,
            after=previous,
            defer_proof=True,
        )
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
                    Script(
                        config.modules.macos_glowup_script,
                        "--content-root",
                        content.root,
                    ),
                    contends=(config.exclusive("apple_vz"),),
                ),
                after=previous,
            ),
        )

    sbom = phase.add(hostpackage.sbom_step(config), after=previous)
    return phase.add(
        install.install_step(config, content=LocalInstallContent(content)),
        after=(sbom,),
    )

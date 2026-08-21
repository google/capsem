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
    installplan,
    platformproof,
)
from .actions import Call, Script
from .command import GateCommand
from .config import GateConfig
from .content import LocalInstallContent, ProfileContent
from .execution import Kind, Needs, Speed, Step, step
from .opacity import CallJustification, OpaqueKind, machine_effects
from .plan import Plan
from .qualification import Qualification
from .staticmodule import storagerelease
from .testmodules import InWorkspace


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

    # Its plan builds an image outside the kernel sandbox, which needs the
    # egress resource to run it with.
    outside_egress = True

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
    staged: ProfileContent | None = None,
) -> Step:
    """Build the release packages and prove an install upgrades cleanly."""
    phase = plan.phase("glowup")
    if qualification.pulled:
        content = staged or ProfileContent.standalone(config)
        return pulled_package(phase, config, qualification, after, content)
    return _build_and_prove(plan, phase, config, after)


# -- the release lane ------------------------------------------------------


def pulled_package(
    phase,
    config: GateConfig,
    qualification: Qualification,
    after: tuple,
    content: ProfileContent,
    *,
    work_dirs: tuple[str, str] | None = None,
    skip_install: bool = False,
    pairing: dict[str, str] | None = None,
) -> Step:
    """The publishable package, proved twice.

    Once as staged, and once with the release environment cleared, so the
    channel switch has to rediscover its state from the installed system
    rather than inherit what it was told.

    Public because `module_rehearsal` runs exactly this against a cohort the
    local lane fabricated from its own build. Not copied there: a rehearsal of
    a different sequence rehearses nothing, and the whole reason it exists is
    that these four steps had never run outside a release dispatch. Only the
    two work directories differ, so that a rehearsal and a real lane on the
    same machine cannot land in one another's scratch.
    """
    settings = config.modules
    glowup_dir, switch_dir = work_dirs or (
        settings.glowup_work_dir,
        settings.channel_switch_work_dir,
    )
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
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=after,
    )
    # Before the install proof, not after it. That proof runs on ubuntu:24.04,
    # which is exactly the glibc the binaries are built against, so it cannot
    # observe a wrong platform floor -- the 0.6.0 package declared no libc at
    # all and passed it. This is the only step that looks below the floor.
    supported = phase.add(
        platformproof.platform_step(config, qualification.package),
        after=(verified,),
    )
    staged = phase.add(
        _glowup_step(
            config,
            "package",
            qualification,
            glowup_dir,
            content,
            skip_install=skip_install,
            pairing=pairing,
        ),
        after=(supported,),
    )
    return phase.add(
        _glowup_step(
            config,
            "channel-switch",
            qualification,
            switch_dir,
            content,
            clear=settings.channel_switch_cleared,
            skip_install=skip_install,
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
    skip_install: bool = False,
    pairing: dict[str, str] | None = None,
) -> Step:
    """The same script the local install proof runs, with the same arguments.

    All three of `--evidence-dir`, `--profile-revision-policy` and the source
    commit were missing here, and every one of them is `required=True`. This
    step could therefore never have got past `argparse` -- a release lane whose
    last two steps were an instant usage error, in the only phase no local run
    reaches. `installproof.prove_glowup` has passed them all along, which is
    what makes the omission invisible: the script is exercised constantly, just
    never through this call site. The rehearsal in `module_rehearsal` exists so
    that stops being true.

    The evidence directory is outside `work_dir` on purpose: the script writes
    its first evidence file and only then clears the work directory, so an
    evidence path underneath would be deleted by the run that wrote it.
    """
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
            "--evidence-dir",
            f"{config.workspace.evidence_dir}/{label}",
            "--profile-revision-policy",
            config.install.profile_revision_policy.value,
            "--package-ready",
            *(("--skip-install",) if skip_install else ()),
            # A workflow exports the pairing; a rehearsal passes it, because it
            # fabricated the cohort and is the only party that knows where it
            # put the two sides. `clear` wins on the channel-switch run, which
            # must rediscover the channel rather than inherit it -- and the two
            # never overlap, because that step is given no pairing at all.
            env={**(pairing or {}), **dict.fromkeys(clear, "")},
            # Installs a system package, so it needs privileges Bubblewrap
            # denies by construction: `PR_SET_NO_NEW_PRIVS` stops sudo dead on
            # a hosted runner. Not the boundary widened for convenience -- it
            # exists to keep a dependency's build script off the network during
            # a compile, and this step compiles nothing. Every compiling step
            # stays inside it, which `test_work_graph_invariants` enforces.
            outside_sandbox=True,
        ),
        contends=(config.exclusive("docker_daemon"),),
        kind=Kind.E2E,
        # NETWORK because it escapes, which the graph invariant holds as one
        # fact stated twice; the installer fetches over loopback and apt
        # resolves runtime dependencies.
        needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.NETWORK}),
        speed=Speed.SLOW,
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
                    kind=Kind.E2E,
                    needs=frozenset({Needs.DOCKER, Needs.DISK}),
                    speed=Speed.SLOW,
                ),
                after=previous,
            ),
        )

    sbom = phase.add(hostpackage.sbom_step(config), after=previous)
    exact_install_image = installplan.fragment(plan, config)
    return phase.add(
        install.install_step(config, content=LocalInstallContent(content)),
        after=(sbom, exact_install_image),
    )

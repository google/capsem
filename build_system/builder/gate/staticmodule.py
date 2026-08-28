"""The source-build proofs: everything provable once something is compiled.

Split from `testmodules`, which was over the three-hundred-line ceiling. The
seam is the one the phases already draw. `fast` holds what needs no build and
answers in seconds; this holds what needs a compiled workspace -- coverage,
doctests, the guest agents, the signed binaries -- and cannot.
"""

from __future__ import annotations

from . import (
    audits,
    host,
    hostpackage,
    imagebases,
    installplan,
    linuxrustimage,
    pytestsuite,
    rustchecks,
    sandbox,
    storage,
    toolchain,
)
from .actions import Run
from .command import GateCommand
from .config import GateConfig
from .egress import Egress
from .execution import Kind, Needs, Speed, Step, step
from .fileactions import RequireFile
from .lifecycle import Resource
from .outside import Outside
from .plan import Plan
from .proc import Runner
from .testmodules import InWorkspace
from .workspace import Workspace


class StaticModule(
    InWorkspace,
    GateCommand,
    name="test-static",
    help="the source-build proofs, before anything boots a VM",
):
    """What can be proved from source, in the order the proofs depend on.

    The install-harness preflight comes first for a blunt reason: proving the
    clean container can launch its runner takes a minute, and discovering it
    cannot after the Rust coverage run wastes twenty.
    """

    outside_egress = True

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return (
            Egress(self._config, enabled=self._sandbox_mode != sandbox.OFF),
            Workspace(self._config),
        )

    def plan(self) -> Plan:
        plan = Plan(self.name)
        static(plan, self._config)
        return plan


def static(
    plan: Plan,
    config: GateConfig,
    *,
    after: tuple[Step, ...] = (),
    generated: Step | None = None,
    bundled: Step | None = None,
) -> tuple[Step, ...]:
    """What can be proved from source, in the order the proofs depend on.

    Returns *every* leaf, not just the last one written. The storage releases
    hang off their own work rather than off the main chain, so a caller that
    waited only for the final step would let the next phase start while a
    release it needs is still outstanding -- which is exactly how the Linux
    parity lane's build tree came to be handed back after the assets had
    already asked for room.
    """
    phase = plan.phase("static")
    settings = config.modules
    leaves: list[Step] = []
    ort = phase.add(toolchain.ort(config, toolchain.OrtConsumer.STATIC), after=after)
    node = phase.add(toolchain.node(config), after=after)
    # Generated once per run. Standalone, this module makes it; composed, it
    # is handed the one the fast phase already made, because the script
    # takes seventy-five seconds and the source it reads has not moved.
    generated = generated or phase.add(audits.generated_settings(config), after=(node,))
    # Shared for the same reason and on the same terms as the settings above.
    frontend = bundled or phase.add(audits.frontend_bundle(config), after=(generated,))

    # Start the install-harness preflight early, but do not make unrelated
    # asset and functional work depend on it.  A retained-prefix refresh can
    # change its source-derived image key; keeping this branch independent
    # means a continuation at a functional step reruns the current image
    # lifecycle instead of carrying an obsolete tag.  Glow-up adds the real
    # consumer edge from smoke to the install transaction.
    installplan.fragment(plan, config, after=after)

    initrd = config.initrd
    host_arch = config.host_arch().name
    guest_builder = phase.add(
        step(
            "guest-builder",
            Outside(imagebases.Prefetch((), rust_names=(host_arch,))),
            Outside(imagebases.MaterializeRustBuilders((host_arch,))),
            contends=(config.exclusive("docker_daemon"),),
            carry_checks=(imagebases.RequireRustBuilders((host_arch,)),),
            kind=Kind.PACKAGE,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=after,
    )
    agents = phase.add(
        step(
            "guest-agents",
            Run(initrd.build),
            # The musl binaries the initrd carries and the VM executes.
            produces=tuple(
                config.path(initrd.staging) / config.host_arch().name / name
                for name in initrd.binaries
            ),
            # `capsem-builder agent` cross-compiles through
            # `builder.docker.cross_compile_agent`, so it drives the daemon.
            # It claimed nothing until the graph invariants noticed: the
            # scheduler was free to run it beside `install.materialize`, which
            # holds the daemon exclusively, and the two would have raced.
            contends=(config.exclusive("docker_daemon"),),
            kind=Kind.COMPILE,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(guest_builder,),
    )
    binaries = phase.add(_guest_binaries_present(config), after=(agents,))
    leaves.append(
        phase.add(
            pytestsuite.Suite(
                label="guest-binary-contracts",
                paths=settings.guest_binary_tests,
                stop_at_first_failure=False,
                require_artifacts=False,
            ).as_step(config),
            after=(binaries,),
        )
    )

    if host.on_macos():
        # Native Linux runs exercise these cfg branches directly. A Mac host
        # has to run the same checked-in Linux runner in Docker, or Linux-only
        # regressions stay out of the local gate entirely.
        #
        # No storage release follows it any more. The lane used to leave an
        # 11 GiB `capsem-linux-rust-target` behind and a step existed to hand it
        # back before the assets needed the room; sealing the lane removed the
        # mount, so the volume, its boundary and that step all went with it.
        leaves.append(linuxrustimage.linux_rust(plan, config, after=after))

    # Both need a built workspace, so both wait on the same three and share
    # one exclusive. They live in `rustchecks` because this module is at the
    # size ceiling the gate holds itself to.
    built = (agents, ort, frontend)
    coverage = phase.add(rustchecks.coverage(config), after=built)
    leaves.append(phase.add(rustchecks.doctests(config), after=built))
    leaves.append(phase.add(hostpackage.sign_step(config), after=(coverage,)))
    return tuple(leaves)


def _guest_binaries_present(config: GateConfig):
    """Every guest binary the host architecture should have produced."""
    root = config.path(config.initrd.staging) / config.host_arch().name
    return step(
        "guest-binaries",
        *[RequireFile(root / name) for name in config.initrd.binaries],
        kind=Kind.STATIC_TEST,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )


def storagerelease(config: GateConfig, phase: str):
    """Hand back the storage a finished rail was holding.

    The one spelling lives in `storage`; this is the name the modules use.
    """
    return storage.release_step(config, phase)

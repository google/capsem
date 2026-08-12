"""The modules `just test` is made of, and the ones provable from source.

`_test-candidate-run` was 366 lines because six modules each re-solved the same
problems inside one `bash` body, selected by a `CAPSEM_TEST_MODULE` environment
variable and a `module_enabled` function. A module was a region of a file
between two `if` statements, which is why running one in isolation meant
setting an environment variable and hoping, and why nothing could say what one
would do without doing it.

Each is a command now. It declares the workspace it needs and the graph of
steps it contains, and both are answerable for free.

`vmmodules` holds the three that cannot start against a bare checkout, because
they need built artifacts or a VM. The seam is what a module needs before it
can begin.
"""

from __future__ import annotations

from . import (
    audits,
    host,
    hostimage,
    hostpackage,
    installimage,
    pytestsuite,
    sandbox,
    sourcechecks,
    storage,
    toolchain,
)
from .actions import Run
from .command import GateCommand
from .config import GateConfig
from .egress import Egress
from .execution import Step, step
from .fileactions import RequireFile
from .lifecycle import Resource
from .plan import Plan
from .proc import Runner
from .workspace import Workspace


class InWorkspace:
    """Runs against an isolated `CAPSEM_HOME`, never the developer's.

    A mixin rather than a base command, because a base would have to register
    itself as a runnable name and there is nothing to run.
    """

    exclusive = True
    sandboxed = sandbox.ENFORCE

    # And against an isolated *checkout*. These are the modules long enough
    # that someone edits the tree while they run -- which has killed four
    # release runs, the last after 61 minutes. An isolated home was never
    # enough on its own: it protects the developer's `~/.capsem` from the gate,
    # and does nothing to protect the gate from the developer.
    private_checkout = True

    _config: GateConfig
    _sandbox_mode: sandbox.SandboxMode
    """Supplied by `GateCommand`, declared here so the mixin type-checks."""

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return (Workspace(self._config),)


class FastModule(
    InWorkspace,
    GateCommand,
    name="test-fast",
    help="the checks that fail in minutes rather than in forty",
):
    """Cheap, independent, and the most common failure class.

    Everything here is free to overlap except one edge: clippy reads
    `frontend/dist`, which `capsem-app` embeds at compile time, so the frontend
    build must finish first. The shell expressed that as a conditional which
    skipped clippy entirely when the frontend failed -- losing the clippy
    result on exactly the runs where the most had changed.
    """

    outside_egress = True

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return (
            Egress(self._config, enabled=self._sandbox_mode != sandbox.OFF),
            Workspace(self._config),
        )

    def plan(self) -> Plan:
        plan = Plan(self.name)
        fast(plan, self._config)
        return plan


def fast(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> tuple[Step, ...]:
    """The cheap checks, returning every independent completion leaf.

    All of them, not whichever was added last. This returned Clippy, and Clippy
    waits on the Rust toolchain and one web surface and on nothing else -- so
    Ruff, both Ty passes, the dependency audits and the other three web
    surfaces gated nothing, and were free to still be running while the gate
    built assets and booted VMs. "The cheap failures run before the expensive
    work" was true of one of them.

    `sourcechecks.fragment` learned this one level down and says so in its own
    docstring; this threw its answer away.
    """
    phase = plan.phase("fast")

    # The environment first: everything below runs through uv or pnpm, and a
    # gate that assumes the lockfile is already installed is a gate that works
    # on the machine it was written on.
    python = phase.add(toolchain.sync(config), after=after)
    node = phase.add(toolchain.node(config), after=(python,))
    rust = phase.add(toolchain.rust(config), after=(python,))
    ort = phase.add(toolchain.ort(config, toolchain.OrtConsumer.FAST), after=(python,))

    # Nothing is worth starting against a file that will not parse.
    syntax = phase.add(audits.source_syntax(config), after=(python,))

    audited = tuple(phase.add(check, after=(syntax,)) for check in audits.all_of(config))
    # The same fragment the `lint` command composes: Ruff and both Ty passes as
    # independent steps, so a Ruff failure no longer hides what Ty would have
    # said and each is timed under its own name.
    checked = sourcechecks.fragment(plan, config, after=(syntax,))
    # Importing every test module is a source-shape proof of the same kind, and
    # the Python counterpart of what `rustinventory` does for nextest: a suite
    # that cannot be collected is a suite the gate would otherwise discover it
    # was not running an hour later.
    collected = phase.add(pytestsuite.collection(config), after=(syntax,))

    # The web surfaces import `frontend/src/lib/mock-settings.generated.ts`,
    # which is gitignored and therefore never part of the source a run is
    # given. This lane only ever *checked* it, so on a warm machine it arrived
    # from an earlier build and on a clean one the frontend check stopped at
    # `Cannot find module './mock-settings.generated'`.
    settings = phase.add(audits.generated_settings(config), after=(python, rust))
    surfaces = [
        phase.add(surface, after=(syntax, node, settings))
        for surface in audits.web_surfaces(config)
    ]
    # One surface is Clippy's prerequisite; the rest are leaves of their own.
    # `settings` needs no entry -- every surface waits on it, so it is already
    # an ancestor of anything that waits on a surface.
    blocking = audits.blocking_surface(config, surfaces)
    clippy = phase.add(audits.clippy(config), after=(blocking, rust, ort))
    return (
        *audited,
        *checked,
        collected,
        *(surface for surface in surfaces if surface is not blocking),
        clippy,
    )


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


def static(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> tuple[Step, ...]:
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

    # Start the install-harness preflight early, but do not make unrelated
    # asset and functional work depend on it.  A retained-prefix refresh can
    # change its source-derived image key; keeping this branch independent
    # means a continuation at a functional step reruns the current image
    # lifecycle instead of carrying an obsolete tag.  Glow-up adds the real
    # consumer edge from smoke to the install transaction.
    installimage.fragment(plan, config, after=after)

    initrd = config.initrd
    agents = phase.add(
        step(
            "guest-agents",
            Run(initrd.build),
            # The musl binaries the initrd carries and the VM executes.
            produces=tuple(
                config.path(initrd.staging) / config.host_arch().name / name
                for name in initrd.binaries
            ),
        ),
        after=after,
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
        leaves.append(hostimage.linux_rust(plan, config, after=after))

    coverage = phase.add(
        step(
            "rust-coverage",
            Run(
                [*settings.rust_coverage, settings.rust_coverage_floor],
                env=toolchain.ort_environment(config, toolchain.OrtConsumer.STATIC),
            ),
            contends=(config.exclusive("workspace_binaries"),),
        ),
        after=(agents, ort),
    )
    leaves.append(phase.add(hostpackage.sign_step(config), after=(coverage,)))
    return tuple(leaves)


def _guest_binaries_present(config: GateConfig):
    """Every guest binary the host architecture should have produced."""
    root = config.path(config.initrd.staging) / config.host_arch().name
    return step(
        "guest-binaries",
        *[RequireFile(root / name) for name in config.initrd.binaries],
    )


def storagerelease(config: GateConfig, phase: str):
    """Hand back the storage a finished rail was holding.

    The one spelling lives in `storage`; this is the name the modules use.
    """
    return storage.release_step(config, phase)

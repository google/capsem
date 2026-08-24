"""The modules `just test-clean` is made of, and the ones provable from source.

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
    digestreport,
    pytestsuite,
    sandbox,
    sourcechecks,
    toolchain,
)
from .command import GateCommand
from .config import GateConfig
from .egress import Egress
from .execution import Kind, Needs, Speed, Step, step
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
    # Readable before the next run finishes; it needs no build output.
    digest = phase.add(step("digest", digestreport.RefreshDigest(),
        kind=Kind.STATIC_TEST,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    ), after=after)
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
    # The Citadel belongs here and not in the broad suite. Source-level guards
    # answering in seconds have no business waiting on an asset build, and the
    # point of recording a mistake is to catch it before the expensive work
    # rather than after the VMs are up.
    guarded = phase.add(pytestsuite.citadel(config).as_step(config), after=(syntax,))

    # The web surfaces import `frontend/src/lib/mock-settings.generated.ts`,
    # which is gitignored and therefore never part of the source a run is
    # given. This lane only ever *checked* it, so on a warm machine it arrived
    # from an earlier build and on a clean one the frontend check stopped at
    # `Cannot find module './mock-settings.generated'`.
    settings = phase.add(audits.generated_settings(config), after=(python, rust))
    # Only the surface that imports the generated mock waits for it.
    #
    # All four used to, and `runs schedule` put the cost on the board: the fast
    # lane's critical path was toolchain -> generated-settings -> release-site,
    # 3m24s, of which the middle 1m11s was a dependency `release-site` does not
    # have. `frontend/src/lib/mock-settings.ts` is the only file in any surface
    # that imports `mock-settings.generated`; docs, site and release-site never
    # touch it. The edge was uniform because it was written once for a list,
    # not because four surfaces needed it.
    #
    # The consumer is the *verify* half, not the surface clippy waits on. Those
    # were one step, so this edge reached clippy transitively and put an
    # `mcp_export` build in front of it for a mock that only `__tests__` files
    # import.
    consumer = config.websurfaces.needs_generated_settings
    surfaces = [
        phase.add(
            surface,
            after=(syntax, node, settings) if surface.label.endswith(consumer) else (syntax, node),
        )
        for surface in audits.web_surfaces(config)
    ]
    # The release-channel parity proof used to be the `release-site` surface's
    # tail. It claims no Astro build, so it no longer stalls the queue.
    channel = phase.add(audits.release_channel(config), after=(syntax, node))
    # One surface is Clippy's prerequisite; the rest are leaves of their own.
    blocking = audits.blocking_surface(config, surfaces)
    clippy = phase.add(audits.clippy(config), after=(blocking, rust, ort))
    return (
        *audited,
        *checked,
        collected,
        guarded,
        digest,
        *(surface for surface in surfaces if surface is not blocking),
        channel,
        clippy,
    )

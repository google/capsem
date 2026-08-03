"""One architecture's package, as steps a graph can order.

`PackageRail` in `packagerail` does the work; this turns its phases into a plan
fragment and registers the command. Every storage-ordering defect in this lane
came from reasoning about the phases from outside one opaque `Call` -- a phase
the graph cannot see is a phase nothing can order, time, or name in a failure.
"""

from __future__ import annotations

import os

from . import hostimage
from .actions import Call, Why
from .command import GateCommand
from .execution import step
from .packagerail import PackageRail
from .plan import Plan


def fragment(plan: Plan, config, target, *, after: tuple = ()):
    """One architecture's package, after the builder image it needs.

    The builder is `shared`, so composing several architectures into one plan
    builds it once and hangs every lane off it.

    `after` reaches the package step and deliberately not the image. The
    glow-up lane chains architectures so the second build waits for the first
    to release its disk; passing that down made the shared image depend on a
    package that depends on the image, which is a cycle -- and one that appears
    only once two lanes share a plan. Groundwork has no ordering of its own.
    """
    built = hostimage.fragment(plan, config)
    phase = plan.phase(f"package.{target.name}")
    docker = (config.exclusive("docker_daemon"),)

    #: The lane, in order. A phase the graph cannot see is a phase nothing can
    #: order, time, or name in a failure -- and every storage-ordering defect
    #: in this file came from reasoning about these six from outside one
    #: opaque `Call`.
    phases = (
        (
            "storage-release",
            "hand back the rails the assets finished with",
            "release_rails",
            Why.DYNAMIC,
        ),
        ("space", "reserve the package rail's headroom", "reserve", Why.COMPUTATION),
        ("clock", "sync the container clock", "sync_clock", Why.DYNAMIC),
        ("sync-assets", f"point the embedded assets at {target.name}", "sync_assets", Why.DYNAMIC),
        # The one instance the class docstring used to describe as though it
        # were all of them: this environment carries the Tauri private key.
        ("build", f"build the Linux release package for {target.name}", "build", Why.SECRETS),
        ("resolve", "read back the exact package the builder recorded", "resolve", Why.COMPUTATION),
        ("prove", "prove that exact package in systemd + KVM", "prove", Why.DYNAMIC),
        ("storage-gc", "list the artifacts and reclaim this lane's disk", "collect", Why.DYNAMIC),
    )

    previous: tuple = (built, *after)
    for label, description, method, why in phases:
        previous = (
            phase.add(
                step(
                    label,
                    Call(description, _phase(target, method), why=why),
                    contends=docker,
                ),
                after=previous,
            ),
        )
    return previous[0]


def _phase(target, method: str):
    """One rail method, as a plan action.

    The rail is rebuilt per phase from the context's runner rather than shared
    across them: a step holding an object an earlier step mutated is a step the
    graph could reorder into nonsense, and the whole point of this shape is
    that the graph *can* reorder them.
    """

    def perform(context) -> None:
        settings = context.config.package
        rail = PackageRail(
            context.runner,
            target,
            manifest_url=os.environ.get(settings.manifest_variable),
            channel=os.environ.get(settings.channel_variable),
            require_proof=os.environ.get(settings.require_proof_variable, "0") == "1",
        )
        getattr(rail, method)()

    return perform


class CrossCompileCommand(
    GateCommand,
    name="cross-compile",
    help="build the Linux release package for one architecture",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("arch", nargs="?", help="arm64 or x86_64; defaults to the host")

    def plan(self) -> Plan:
        config = self._config
        target = config.arch(self._args.arch) if self._args.arch else config.host_arch()
        plan = Plan(self.name)
        fragment(plan, config, target)
        return plan

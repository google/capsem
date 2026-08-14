"""One architecture's package, as steps a graph can order.

`PackageRail` in `packagerail` does the work; this turns its phases into a plan
fragment and registers the command. Every storage-ordering defect in this lane
came from reasoning about the phases from outside one opaque `Call` -- a phase
the graph cannot see is a phase nothing can order, time, or name in a failure.
"""

from __future__ import annotations

import os
from pathlib import Path

from . import hostimage, installplan
from .actions import Call
from .command import GateCommand
from .content import ProfileContent
from .execution import Kind, Needs, Speed, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .packagerail import PackageRail
from .plan import Plan

#: One reason per phase. The class docstring used to carry a single rationale
#: -- "a package build carries signing material" -- for all eight, which is
#: true of exactly one of them.
RAILS = "which rails the assets finished with is resolved from the policy at run time"
HEADROOM = "reads the daemon's free space before an hour of compilation spends it"
CLOCK = "Colima's clock drift is a property of the machine this runs on"
CONTENT = "verifies one paired asset and configuration bundle for the package target"
MATERIALIZE = "resolves locked package dependencies at the explicit network-open boundary"
SIGNING = "its environment carries the Tauri private key, which a dry run must not print"
RECORDED = "reads back the exact package basename the builder just wrote"
PROOF = "installs that package in a systemd container and proves what it produced"
DELEGATED_PROOF = (
    "the complete candidate's local install transaction owns a stronger authoritative proof"
)
RECLAIM = "what this lane left on disk is only knowable once it has finished"


def _because(reason: str, *effects: Effect) -> CallJustification:
    """One phase's justification, with `SECRETS` the only special case."""
    kind = (
        OpaqueKind.SECRET_BEARING
        if reason is SIGNING
        else OpaqueKind.PURE_INSPECTION
        if not effects or effects == (Effect.PROCESS,)
        else OpaqueKind.RUNTIME_DERIVED
    )
    return CallJustification(kind=kind, reason=reason, effects=machine_effects(*effects))


def fragment(
    plan: Plan,
    config,
    target,
    *,
    content: ProfileContent,
    after: tuple = (),
    defer_proof: bool = False,
):
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

    proof = (
        (
            "defer exact package proof to the local install transaction",
            "defer_proof",
            _because(DELEGATED_PROOF),
        )
        if defer_proof
        else (
            "prove that exact package in systemd + KVM",
            "prove",
            _because(PROOF, Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
        )
    )

    #: The lane, in order. A phase the graph cannot see is a phase nothing can
    #: order, time, or name in a failure -- and every storage-ordering defect
    #: in this file came from reasoning about these six from outside one
    #: opaque `Call`.
    phases = (
        (
            "storage-release",
            "hand back the rails the assets finished with",
            "release_rails",
            _because(RAILS, Effect.PROCESS, Effect.HOST_STATE),
        ),
        (
            "space",
            "reserve the package rail's headroom",
            "reserve",
            _because(HEADROOM, Effect.PROCESS),
        ),
        (
            "clock",
            "sync the container clock",
            "sync_clock",
            _because(CLOCK, Effect.PROCESS),
        ),
        (
            "content",
            f"verify paired package content for {target.name}",
            "require_content",
            _because(CONTENT),
        ),
        (
            "materialize",
            f"materialize locked package dependencies for {target.name}",
            "materialize",
            _because(MATERIALIZE, Effect.PROCESS, Effect.HOST_STATE, Effect.NETWORK),
        ),
        # The one instance the class docstring used to describe as though it
        # were all of them: this environment carries the Tauri private key.
        (
            "build",
            f"build the Linux release package for {target.name}",
            "build",
            _because(SIGNING, Effect.PROCESS, Effect.FILESYSTEM),
        ),
        (
            "resolve",
            "read back the exact package the builder recorded",
            "resolve",
            _because(RECORDED),
        ),
        (
            "prove",
            proof[0],
            proof[1],
            proof[2],
        ),
        (
            "storage-gc",
            "list the artifacts and reclaim this lane's disk",
            "collect",
            _because(RECLAIM, Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
        ),
    )

    previous: tuple = (built, *after)
    for label, description, method, justification in phases:
        previous = (
            phase.add(
                step(
                    label,
                    Call(
                        description,
                        _phase(target, method, content),
                        justification=justification,
                    ),
                    contends=docker,
                    kind=Kind.COMPILE,
                    needs=frozenset({Needs.DOCKER, Needs.DISK}),
                    speed=Speed.SLOW,
                ),
                after=previous,
            ),
        )
    return previous[0]


def _phase(target, method: str, content: ProfileContent):
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
            content=content,
            manifest_url=os.environ.get(settings.manifest_variable),
            channel=os.environ.get(settings.channel_variable),
            require_proof=os.environ.get(settings.require_proof_variable, "0") == "1",
        )
        result = getattr(rail, method)()
        if method == "materialize":
            context.journal.note(
                f"package helper {target.name}: input key {result.input_key}; "
                f"exact image {result.image_id}; build reference {result.image_reference}"
            )

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
        parser.add_argument(
            "--content-root",
            help="paired assets/config root already selected by a release or candidate rail",
        )
        parser.add_argument(
            "--defer-proof",
            action="store_true",
            help="defer exact package proof to the release install transaction",
        )

    def plan(self) -> Plan:
        config = self._config
        target = config.arch(self._args.arch) if self._args.arch else config.host_arch()
        content_root = getattr(self._args, "content_root", None)
        defer_proof = bool(getattr(self._args, "defer_proof", False))
        if defer_proof and content_root is None:
            from .errors import GateError

            raise GateError("--defer-proof requires an explicit --content-root")
        if content_root is None:
            content = ProfileContent.standalone(config)
        else:
            selected = Path(content_root)
            content = ProfileContent.isolated(
                config, selected if selected.is_absolute() else config.path(str(selected))
            )
        plan = Plan(self.name)
        # The complete candidate composes `install-image` in its static phase
        # before it reaches the package lanes. This standalone command has no
        # such caller, yet a native package's `prove` phase boots the exact deb
        # in that image. Own the prerequisite here so a clean machine cannot
        # build the package and then fail by trying to pull our local-only
        # `capsem-install-test` tag from a registry.
        after = ()
        if target == config.host_arch() and not defer_proof:
            after = (installplan.fragment(plan, config),)
        fragment(
            plan,
            config,
            target,
            content=content,
            after=after,
            defer_proof=defer_proof,
        )
        return plan

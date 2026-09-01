"""Composing the install proof into a plan, and the command that runs it.

Split from `installimage`, which builds and identifies the image and was over
the three-hundred-line ceiling. The dependency runs one way -- planning needs
the image, the image knows nothing about the plan -- which is the seam that
does not become a cycle.
"""

from __future__ import annotations

from . import hostimage, installbuilder
from .actions import Call
from .cachecontrol import CacheControl
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Needs, Requires, Speed, Step, step
from .installimage import (
    InstallImageStep,
    RequireInstallImage,
    _smoke,
    _step_label,
    build_source_image,
    require_local_image,
)
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .outside import Outside
from .plan import Plan
from .sourcecapture import require_recorded
from .sourcestate import record_step


def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Expose the sole egress edge and both sealed phases in the gate graph."""
    # An existing candidate source boundary already precedes `after`; adding
    # those later steps back onto it would create a cycle. Standalone plans
    # still get the same root step here.
    recorded = plan.shared(record_step(config))
    built = hostimage.fragment(plan, config, after=after)

    capacity = plan.shared(
        step(
            _step_label(InstallImageStep.CAPACITY),
            Call(
                "reserve disk for the install helper and exact source image",
                lambda context: CacheControl(context.runner).ensure_space("install-preflight"),
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="the storage policy measures and reserves Docker capacity",
                    effects=machine_effects(Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
                ),
            ),
            contends=(config.exclusive("docker_daemon"),),
            kind=Kind.E2E,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(built, *after),
    )

    def materialize(context) -> None:
        identity = installbuilder.materialize(context.runner, context.config)
        context.journal.note(
            f"install helper: input key {identity.input_key}; exact image {identity.image_id}; "
            f"build reference {identity.image_reference}"
        )

    materialized = plan.shared(
        step(
            _step_label(InstallImageStep.MATERIALIZE),
            Outside(
                Call(
                    "materialize locked install qualification dependencies",
                    materialize,
                    justification=CallJustification(
                        kind=OpaqueKind.RUNTIME_DERIVED,
                        reason="the exact host-builder child and helper input key resolve at run time",
                        effects=machine_effects(
                            Effect.PROCESS,
                            Effect.FILESYSTEM,
                            Effect.NETWORK,
                            Effect.HOST_STATE,
                        ),
                    ),
                )
            ),
            contends=(config.exclusive("docker_daemon"),),
            kind=Kind.E2E,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(capacity,),
    )
    # This is the host builder's real byte consumer. The capacity step merely
    # orders disk reservation. Once materialization is carried, bounded GC may
    # have reclaimed the working parent without invalidating a later resume.
    plan.edge(before=built, after=materialized, requires=Requires.ARTIFACT)

    def build(context) -> None:
        helper = installbuilder.require_current(context.runner, context.config)
        source = require_recorded(context.config)
        identity = build_source_image(
            context.runner,
            context.config,
            identity=helper,
            source=source,
        )
        context.journal.note(
            f"install image: input key {identity.input_key}; exact image {identity.image_id}; "
            f"build reference {identity.image_reference}"
        )

    image = plan.shared(
        step(
            _step_label(InstallImageStep.BUILD),
            Call(
                "build the network-denied install qualification image",
                build,
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="helper identity and source digest resolve at run time",
                    effects=machine_effects(Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
                ),
            ),
            contends=(config.exclusive("docker_daemon"),),
            kind=Kind.E2E,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
            produces=(config.path(config.install.builder.source_identity_file),),
            carry_checks=(RequireInstallImage(),),
        ),
        after=(materialized, recorded),
    )

    def smoke(context) -> None:
        exact = require_local_image(context.runner, context.config)
        _smoke(context.runner, context.config, image=exact)
        context.journal.note(f"sealed install image smoke passed: exact image {exact}")

    return plan.shared(
        step(
            _step_label(InstallImageStep.SMOKE),
            Call(
                "smoke the exact install image with networking denied",
                smoke,
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="the exact source image ID is revalidated immediately before smoke",
                    effects=machine_effects(Effect.PROCESS, Effect.HOST_STATE),
                ),
            ),
            contends=(config.exclusive("docker_daemon"),),
            kind=Kind.E2E,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(image,),
    )


class InstallImageCommand(
    GateCommand,
    name="install-image",
    help="materialize and smoke the sealed install qualification image",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        fragment(plan, self._config)
        return plan

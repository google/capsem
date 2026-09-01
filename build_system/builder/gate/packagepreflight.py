"""Qualify package-network dependencies before expensive candidate work."""

from __future__ import annotations

from . import hostimage, packagebuilder
from .actions import Call
from .config import Arch, GateConfig
from .context import Context
from .execution import Kind, Needs, Requires, Speed, Step, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan


def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> Step:
    """Materialize both helpers before assets, VMs, and broad tests.

    The helper is input-keyed, so the later package rail only verifies and
    reuses it. Serial ordering makes the first unavailable immutable snapshot
    stop the candidate without launching a sibling download attempt.
    """
    builder = hostimage.fragment(plan, config, after=after)
    phase = plan.phase("prepare.package-dependencies")
    previous: Step | None = None
    for target in config.architectures.values():
        dependency = phase.add(
            step(
                target.name,
                Call(
                    f"materialize locked package dependencies for {target.name} early",
                    _materialize(target),
                    justification=CallJustification(
                        kind=OpaqueKind.RUNTIME_DERIVED,
                        reason=(
                            "resolves the exact package helper identity at the early "
                            "network qualification boundary"
                        ),
                        effects=machine_effects(
                            Effect.PROCESS,
                            Effect.NETWORK,
                            Effect.HOST_STATE,
                        ),
                    ),
                ),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.COMPILE,
                needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(() if previous is None else (previous,)),
            requires=Requires.ORDER,
        )
        plan.edge(before=builder, after=dependency, requires=Requires.ARTIFACT)
        previous = dependency
    if previous is None:  # Pydantic validation normally makes this unreachable.
        raise ValueError("candidate package dependency inventory is empty")
    return previous


def _materialize(target: Arch):
    def perform(context: Context) -> None:
        identity = packagebuilder.materialize(context.runner, context.config, target)
        context.journal.note(
            f"early package helper {target.name}: input key {identity.input_key}; "
            f"exact image {identity.image_id}; build reference {identity.image_reference}"
        )

    return perform

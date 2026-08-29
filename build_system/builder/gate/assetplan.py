"""The asset gate, as the four steps a graph can order.

`AssetGate` in `assets` does the work -- preflight, one lane per architecture,
the sweep, and the assembly. This turns those into a plan fragment and
registers the command, which is the same seam `packagerail`/`crosscompile` and
`debproof`/`debproofcommand` sit on: what a phase *does* against a machine and
how a graph *orders* it change for different reasons.

The two lanes hold Docker `shared`, because they must overlap to fit a hosted
runner's lifetime, while every other Docker step holds it exclusively. That was
a thread pool the plan could not see, order against, or attribute a failure to.
"""

from __future__ import annotations

from dataclasses import replace

from . import assetdependencies, assetreceipt, hostpackage, initrd
from .actions import Call
from .assetlanes import RequireLaneReceipts, SealPackedReceipts, discover_profiles, lane_assets
from .assets import AssetGate
from .command import GateCommand
from .execution import Kind, Needs, Speed, step
from .imagebases import (
    MaterializeAssetTools,
    MaterializeRustBuilders,
    Prefetch,
    RequireAssetTools,
    RequireRustBuilders,
    required_rust_builder_names,
)
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan

#: One reason per phase of the asset gate. Hoisted so the plan below reads as
#: a plan rather than as four paragraphs of justification.
PREFLIGHT = "reads daemon capacity and clears derived output while retaining isolated lane caches"
LANE = "one architecture's build lane: several builder invocations that only mean anything together"
SWEEP = "which containers the lanes left behind is only knowable once they have run"
ASSEMBLE = "merge, publish, materialise and boot each profile, as one indivisible assembly"


def _because(kind: OpaqueKind, reason: str, *effects: Effect) -> CallJustification:
    return CallJustification(kind=kind, reason=reason, effects=machine_effects(*effects))


def fragment(plan, config, *, after: tuple = ()):
    """The asset phase, as steps the plan can see.

    `sweep` runs after both lanes whatever they did, because the scheduler
    skips a step only when something it depends on failed -- and this depends
    on the lanes finishing, not on their succeeding.
    """
    phase = plan.phase("assets")
    exclusive = (config.exclusive("docker_daemon"),)
    shared = (config.shared("docker_daemon"),)
    rust_builders = required_rust_builder_names(config)
    profiles = discover_profiles(config)

    ready = phase.add(
        step(
            "preflight",
            Prefetch(rust_names=rust_builders, asset_tools=True),
            Call(
                "run container preflight, check capacity, and retain only isolated lane caches",
                lambda ctx: AssetGate(ctx.runner).preflight(),
                justification=_because(
                    OpaqueKind.RUNTIME_DERIVED,
                    PREFLIGHT,
                    Effect.PROCESS,
                    Effect.FILESYSTEM,
                ),
            ),
            MaterializeRustBuilders(rust_builders),
            MaterializeAssetTools(),
            contends=exclusive,
            carry_checks=(RequireRustBuilders(rust_builders), RequireAssetTools()),
            kind=Kind.PACKAGE,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=after,
    )
    dependencies = phase.add(
        assetdependencies.dependency_step(
            config,
            (profile.name for profile in profiles),
            config.architectures,
            config.imagebuild.lane_templates,
        ),
        after=(ready,),
    )
    lanes = tuple(
        phase.add(
            step(
                f"build.{name}",
                Call(
                    f"build every profile's assets for {name}",
                    _lane(name),
                    justification=_because(
                        OpaqueKind.DOMAIN_TRANSACTION,
                        LANE,
                        Effect.PROCESS,
                        Effect.FILESYSTEM,
                    ),
                ),
                contends=shared,
                carry_checks=(
                    RequireLaneReceipts(
                        config,
                        profiles,
                        (config.arch(name),),
                    ),
                ),
                kind=Kind.PACKAGE,
                needs=frozenset({Needs.DOCKER, Needs.DISK}),
                speed=Speed.SLOW,
            ),
            after=(dependencies,),
        )
        for name in config.architectures
    )
    swept = phase.add(
        step(
            "sweep",
            Call(
                "remove containers the lanes left",
                lambda ctx: AssetGate(ctx.runner).sweep(),
                justification=_because(
                    OpaqueKind.RUNTIME_DERIVED,
                    SWEEP,
                    Effect.PROCESS,
                    Effect.HOST_STATE,
                ),
            ),
            contends=exclusive,
            kind=Kind.PACKAGE,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=lanes,
    )
    targets = {
        name: tuple(
            lane_assets(config, profile, config.arch(name)) / name / config.artifacts.initrd
            for profile in profiles
        )
        for name in config.architectures
    }
    packed_step = initrd.repack_step(config, targets)
    packed = phase.add(
        replace(
            packed_step,
            actions=(*packed_step.actions, SealPackedReceipts(config, profiles)),
            carry_checks=(
                RequireLaneReceipts(
                    config,
                    profiles,
                    tuple(config.architectures.values()),
                    stages=assetreceipt.PACKED_STAGES,
                ),
            ),
        ),
        after=(swept,),
    )
    built = phase.add(
        hostpackage.build_step(config, label="build-host-binaries"),
        after=(packed,),
    )
    signed = phase.add(
        hostpackage.sign_step(config, label="sign-host-binaries"),
        after=(built,),
    )
    return phase.add(
        step(
            "assemble",
            Call(
                "merge, publish, materialise and boot each profile",
                lambda ctx: AssetGate(ctx.runner).assemble(),
                justification=_because(
                    OpaqueKind.DOMAIN_TRANSACTION,
                    ASSEMBLE,
                    Effect.PROCESS,
                    Effect.FILESYSTEM,
                ),
            ),
            contends=exclusive,
            kind=Kind.PACKAGE,
            needs=frozenset({Needs.DOCKER, Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(signed,),
    )


def _lane(arch_name: str):
    def perform(context) -> None:
        AssetGate(context.runner).lane(arch_name)

    return perform


class AssetsCommand(
    GateCommand, name="assets", help="build every profile's VM assets and boot each one"
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        fragment(plan, self._config)
        return plan

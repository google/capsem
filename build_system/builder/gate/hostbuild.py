"""The bounded host-binary build fragment and its Cargo cache prerequisite."""

from __future__ import annotations

from .actions import Call, Run
from .cachecontrol import CacheControl
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .phase import Phase
from .plan import Plan


def _build_step(
    config: GateConfig, *, label: str = "build-binaries", env: dict[str, str] | None = None
) -> Step:
    """Build exactly the binaries the signing step owns.

    They once had no producer: signing rewrote whatever an earlier build left
    in the shared target and failed on a clean machine. The configured built
    set keeps production and signing from drifting apart.
    """
    settings = config.signing
    selected = [flag for name in settings.built for flag in ("--bin", name)]
    binary_dir = config.path(settings.binaries[0]).parent
    return step(
        label,
        Run(
            ["cargo", "build", *selected],
            env=env,
            timeout_seconds=settings.build_timeout_seconds,
        ),
        contends=(config.exclusive("workspace_binaries"),),
        produces=tuple(binary_dir / name for name in settings.built),
        kind=Kind.PACKAGE,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
    )


def _cargo_cache_step(config: GateConfig) -> Step:
    return step(
        "cargo-cache-enforcement",
        Call(
            "enforce the Cargo cache maximum before host compilation",
            lambda ctx: CacheControl(ctx.runner).enforce("cargo", "host compilation"),
            justification=CallJustification(
                kind=OpaqueKind.RUNTIME_DERIVED,
                reason="the typed cache owner inventories shared compilation units at runtime",
                effects=machine_effects(Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
            ),
        ),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.PACKAGE,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )


def add(
    owner: Plan | Phase,
    config: GateConfig,
    *,
    after: tuple[Step, ...] = (),
    label: str = "build-binaries",
    env: dict[str, str] | None = None,
    cache_already_enforced: bool = False,
) -> Step:
    """Add the one host build path, with visible cache enforcement first.

    Complete qualification already enforces every configured cache before it
    reaches this fragment. Every focused composer takes the default and gets a
    separate timed prerequisite, so the shared Cargo target cannot silently
    grow past its contract again.
    """
    dependencies = after
    if not cache_already_enforced:
        bounded = owner.add(_cargo_cache_step(config), after=after)
        dependencies = (bounded,)
    return owner.add(_build_step(config, label=label, env=env), after=dependencies)

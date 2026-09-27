"""The host-binary build fragment."""

from __future__ import annotations

from .actions import Run
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .phase import Phase
from .plan import Plan


def _build_step(config: GateConfig, *, label: str = "build-binaries") -> Step:
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
            timeout_seconds=settings.build_timeout_seconds,
        ),
        contends=(config.exclusive("workspace_binaries"),),
        produces=tuple(binary_dir / name for name in settings.built),
        kind=Kind.PACKAGE,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
    )


def add(
    owner: Plan | Phase,
    config: GateConfig,
    *,
    after: tuple[Step, ...] = (),
    label: str = "build-binaries",
) -> Step:
    """Add the one host build path.

    The shared Cargo target's maximum is held by the command, not here:
    `CargoCacheBound` enforces it before the first and after the last step of
    every plan that compiles, which a prerequisite of this one fragment could
    not do for clippy, coverage, or any other compile beside it.
    """
    return owner.add(_build_step(config, label=label), after=after)

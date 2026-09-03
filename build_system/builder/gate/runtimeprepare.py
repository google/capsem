"""One typed preparation fragment for source-built host and VM runtimes."""

from __future__ import annotations

from dataclasses import dataclass

from . import hostpackage, imagebuild, initrd
from .actions import Run
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan


@dataclass(frozen=True)
class Preparation:
    """Canonical runtime and the profile content it materialized."""

    ready: Step
    profile_content: Step


def prepare(
    plan: Plan,
    config: GateConfig,
    *,
    after: tuple[Step, ...],
    guest: bool = True,
    build_label: str = "build-binaries",
    sign_label: str = "sign",
) -> Preparation:
    """Build one self-contained runtime, optionally including VM inputs."""
    phase = plan.phase("prepare")
    previous = after
    if guest:
        assets = imagebuild.check_assets(
            plan,
            config,
            after=after,
            doctor_skips=dict(config.candidate.doctor_skips),
        )
        packed = initrd.pack(plan, config, after=assets)
        previous = (packed,)

    materialized = phase.add(materialize_config_step(config), after=previous)
    built = phase.add(hostpackage.build_step(config, label=build_label), after=(materialized,))
    ready = phase.add(hostpackage.sign_step(config, label=sign_label), after=(built,))
    return Preparation(ready=ready, profile_content=materialized)


def materialize_config_step(config: GateConfig) -> Step:
    """Produce the canonical config half of locally built profile content."""
    return step(
        "materialize-config",
        Run(["bash", config.candidate.materialize_script]),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )

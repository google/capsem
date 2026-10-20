"""One typed preparation fragment for source-built host and VM runtimes."""

from __future__ import annotations

from dataclasses import dataclass

from . import assetrecovery, hostbuild, hostpackage, initrd
from .config import GateConfig
from .execution import Step
from .plan import Plan
from .rebuildpermission import DEFAULT_PERMISSION, RebuildPermission


@dataclass(frozen=True)
class Preparation:
    """The canonical runtime, signed and ready."""

    ready: Step


def prepare(
    plan: Plan,
    config: GateConfig,
    *,
    after: tuple[Step, ...] = (),
    guest: bool = True,
    permission: RebuildPermission = DEFAULT_PERMISSION,
    build_label: str = "build-binaries",
    sign_label: str = "sign",
) -> Preparation:
    """Build one self-contained runtime, optionally including VM inputs."""
    phase = plan.phase("prepare")
    previous = after
    if guest:
        assets = assetrecovery.check_assets(
            plan,
            config,
            permission=permission,
            after=after,
            doctor_skips=dict(config.candidate.doctor_skips),
        )
        packed = initrd.pack(plan, config, after=assets)
        previous = (packed,)

    built = hostbuild.add(phase, config, after=previous, label=build_label)
    ready = phase.add(hostpackage.sign_step(config, label=sign_label), after=(built,))
    return Preparation(ready=ready)


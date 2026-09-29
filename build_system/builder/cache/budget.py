"""The disk budget: every cache maximum together, plus headroom, fits the disk.

Each cache's contract bounds that cache alone. On 2026-09-29 the maxima summed
to 839 GiB on a 484 GB build box, so every cache could honour its own contract
while the disk filled and ENOSPC killed gate and release runs. The budget is
the contract between them: the policy declares the smallest filesystem a
retained-cache machine may have, loading refuses maxima that do not fit it,
and the gate refuses a machine whose filesystem cannot hold them.
"""

from __future__ import annotations

import os
import shutil
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import TYPE_CHECKING, Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictFloat, StrictInt

if TYPE_CHECKING:
    from .models import CachePolicy

GIB = 1024**3
_NAMED = 5


class BudgetError(RuntimeError):
    """This machine cannot hold every cache at its maximum."""


class BudgetPolicy(BaseModel):
    """What share of a filesystem the caches may claim together."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    #: The share of the cache filesystem that all maxima plus headroom may use.
    filesystem_fraction: Annotated[StrictFloat, Field(gt=0, le=1)]
    #: Room for everything a maximum does not bound: a build's peak, the OS.
    headroom_bytes: Annotated[StrictInt, Field(ge=0)]
    #: The smallest filesystem a retained-cache machine may have; loading
    #: refuses maxima that would not fit it.
    minimum_filesystem_bytes: Annotated[StrictInt, Field(gt=0)]
    #: Variables naming a machine whose caches die with it, such as a
    #: GitHub-hosted runner. Its disk is its image's concern, not this policy's.
    ephemeral_environment: dict[str, str]


def maxima(policy: CachePolicy, present: Callable[[str], bool] | None = None) -> dict[str, int]:
    """Every independently bounded owner's maximum: stages and runtimes.

    Docker image repositories are children of the Docker runtime and already
    inside its maximum, so they are not counted twice. Given `present`, an
    optional runtime whose command is absent holds nothing (Tart on Linux).
    """
    return {
        **{name: stage.max_size_bytes for name, stage in policy.stages.items()},
        **{
            name: runtime.max_size_bytes
            for name, runtime in policy.runtimes.items()
            if present is None or runtime.required or present(runtime.command)
        },
    }


def total_maxima(policy: CachePolicy, present: Callable[[str], bool] | None = None) -> int:
    return sum(maxima(policy, present).values())


def _installed(command: str) -> bool:
    return shutil.which(command) is not None


def check_policy(policy: CachePolicy) -> None:
    """Refuse maxima that do not fit the declared smallest machine."""
    limits = policy.budget
    if limits is None:
        return
    allowed = limits.filesystem_fraction * limits.minimum_filesystem_bytes
    total = total_maxima(policy)
    if total + limits.headroom_bytes > allowed:
        raise ValueError(
            f"cache maxima sum to {_gib(total)} plus {_gib(limits.headroom_bytes)} headroom, "
            f"above {limits.filesystem_fraction:g} of the "
            f"{_gib(limits.minimum_filesystem_bytes)} minimum filesystem ({_gib(int(allowed))}); "
            f"largest: {_largest(policy)}"
        )


def verify_machine(
    policy: CachePolicy,
    *,
    filesystem_bytes: int,
    environment: Mapping[str, str],
    present: Callable[[str], bool] = _installed,
) -> None:
    """Refuse a retained-cache machine that cannot hold every cache at once."""
    limits = policy.budget
    if limits is None:
        raise BudgetError("the cache policy declares no disk budget ([budget] in config/cache.toml)")
    if any(environment.get(name) == value for name, value in limits.ephemeral_environment.items()):
        return
    total = total_maxima(policy, present)
    allowed = limits.filesystem_fraction * filesystem_bytes
    if total + limits.headroom_bytes > allowed:
        raise BudgetError(
            f"{_gib(total)} of cache maxima + {_gib(limits.headroom_bytes)} headroom do not fit "
            f"{limits.filesystem_fraction:g} of this {_gib(filesystem_bytes)} filesystem "
            f"({_gib(int(allowed))}), so the caches can fill the disk while each stays within "
            f"its own contract. Largest: {_largest(policy, present)}. Lower maxima in "
            "config/cache.toml, or run on a larger disk."
        )


def filesystem_bytes(path: Path) -> int:
    """The size of the filesystem holding `path`, used or not."""
    stats = os.statvfs(path)
    return stats.f_blocks * stats.f_frsize


def _largest(policy: CachePolicy, present: Callable[[str], bool] | None = None) -> str:
    ranked = sorted(maxima(policy, present).items(), key=lambda item: (-item[1], item[0]))
    return ", ".join(f"{name} {_gib(size)}" for name, size in ranked[:_NAMED])


def _gib(size: int) -> str:
    return f"{size / GIB:.1f} GiB"

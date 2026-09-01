"""Typed capacity assessment for one cache inventory."""

from __future__ import annotations

from enum import StrEnum

from pydantic import BaseModel, ConfigDict, StrictBool, StrictInt

from .models import CacheInventory, CachePolicy
from .render import bytes_label


class Pressure(StrEnum):
    OK = "ok"
    WARNING = "warning"
    SOFT = "soft"
    HARD = "hard"


class StageHealth(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    stage_id: str
    pressure: Pressure
    logical_bytes: StrictInt
    warning_bytes: StrictInt
    soft_bytes: StrictInt
    hard_bytes: StrictInt
    managed_count: StrictInt
    maximum_count: StrictInt | None


class CacheHealth(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    healthy: StrictBool
    filesystem_free_bytes: StrictInt
    minimum_free_bytes: StrictInt
    stages: tuple[StageHealth, ...]
    violations: tuple[str, ...]


def _pressure(size: int, warning: int, soft: int, hard: int) -> Pressure:
    if size > hard:
        return Pressure.HARD
    if size > soft:
        return Pressure.SOFT
    if size > warning:
        return Pressure.WARNING
    return Pressure.OK


def assess(inventory: CacheInventory, policy: CachePolicy) -> CacheHealth:
    """Interpret configured limits without mutating or inventing retention."""
    stages = []
    violations = []
    for stage in inventory.stages:
        configured = policy.stages[stage.stage_id]
        managed_count = sum(entry.managed for entry in stage.entries)
        pressure = _pressure(
            stage.logical_bytes,
            configured.warning_bytes,
            configured.soft_bytes,
            configured.hard_bytes,
        )
        if pressure is Pressure.HARD:
            violations.append(
                f"{stage.stage_id} uses {stage.logical_bytes} bytes above hard cap "
                f"{configured.hard_bytes}"
            )
        if configured.maximum_count is not None and managed_count > configured.maximum_count:
            violations.append(
                f"{stage.stage_id} has {managed_count} managed generations above count cap "
                f"{configured.maximum_count}"
            )
            if pressure in {Pressure.OK, Pressure.WARNING}:
                pressure = Pressure.SOFT
        stages.append(
            StageHealth(
                stage_id=stage.stage_id,
                pressure=pressure,
                logical_bytes=stage.logical_bytes,
                warning_bytes=configured.warning_bytes,
                soft_bytes=configured.soft_bytes,
                hard_bytes=configured.hard_bytes,
                managed_count=managed_count,
                maximum_count=configured.maximum_count,
            )
        )
    if inventory.filesystem_free_bytes < policy.minimum_free_bytes:
        violations.append(
            f"filesystem has {inventory.filesystem_free_bytes} free bytes below reserve "
            f"{policy.minimum_free_bytes}"
        )
    return CacheHealth(
        healthy=not violations,
        filesystem_free_bytes=inventory.filesystem_free_bytes,
        minimum_free_bytes=policy.minimum_free_bytes,
        stages=tuple(stages),
        violations=tuple(violations),
    )


def render(report: CacheHealth) -> str:
    lines = [
        f"Cache health: {'OK' if report.healthy else 'ACTION REQUIRED'}",
        f"Free: {bytes_label(report.filesystem_free_bytes)} "
        f"(reserve {bytes_label(report.minimum_free_bytes)})",
    ]
    lines.extend(
        f"  {stage.stage_id}: {stage.pressure} -- {bytes_label(stage.logical_bytes)}, "
        f"{stage.managed_count} managed"
        for stage in report.stages
        if stage.pressure is not Pressure.OK
    )
    lines.extend(f"  VIOLATION: {violation}" for violation in report.violations)
    return "\n".join(lines)

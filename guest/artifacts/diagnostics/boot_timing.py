"""Deterministic assessment of capsem-init stage timings."""

from dataclasses import dataclass
from typing import Any


MAX_BOOT_STAGE_MS = 500


@dataclass(frozen=True)
class BootTimingAssessment:
    """Aggregate timing for reporting plus attributable stage regressions."""

    total_ms: int
    slow_stages: tuple[dict[str, Any], ...]


def assess_boot_timing(stages: list[dict[str, Any]]) -> BootTimingAssessment:
    """Return total time and stages that exceed the product stage budget.

    A shared host can deschedule the guest across several otherwise healthy
    stages, so the aggregate is diagnostic rather than a deterministic gate.
    A single stage crossing the budget remains attributable to one init
    operation and therefore blocks the doctor gate.
    """
    total_ms = sum(int(stage.get("duration_ms", 0)) for stage in stages)
    slow_stages = tuple(
        stage
        for stage in stages
        if int(stage.get("duration_ms", 0)) > MAX_BOOT_STAGE_MS
    )
    return BootTimingAssessment(total_ms=total_ms, slow_stages=slow_stages)

"""Deterministic assessment of capsem-init stage timings."""

import math
from dataclasses import dataclass
from typing import Any

MAX_BOOT_STAGE_MS = 500
# Reported for the total but not budgeted per stage: the kernel is not an
# init operation anyone can attribute a regression to from inside the guest.
UNBUDGETED_STAGES = frozenset({"kernel"})


@dataclass(frozen=True)
class BootTimingAssessment:
    """Aggregate timing for reporting plus attributable stage regressions."""

    total_ms: int
    slow_stages: tuple[dict[str, Any], ...]


def _steal_ms(stage: dict[str, Any]) -> int:
    """The stage's recorded steal, or 0 when absent or not a sane count.

    Anything that is not a finite non-negative integer discounts nothing, so a
    corrupt or hostile line can only make the budget stricter, never looser.
    """
    value = stage.get("steal_ms", 0)
    if isinstance(value, bool):
        return 0
    if isinstance(value, float):
        value = int(value) if math.isfinite(value) else 0
    elif isinstance(value, str):
        value = int(value) if value.isdecimal() else 0
    return value if isinstance(value, int) and value > 0 else 0


def _guest_work_ms(stage: dict[str, Any]) -> int:
    """Wall time of the stage minus the host steal accrued during it."""
    duration_ms = int(stage.get("duration_ms", 0))
    return duration_ms - min(_steal_ms(stage), max(duration_ms, 0))


def assess_boot_timing(stages: list[dict[str, Any]]) -> BootTimingAssessment:
    """Return total wall time and stages whose guest work exceeds the budget.

    A shared host can deschedule the guest across several otherwise healthy
    stages, so the aggregate is diagnostic rather than a deterministic gate.
    One long deschedule can also land inside a single stage, so each stage is
    budgeted on its wall time minus the steal capsem-init recorded for it
    (``steal_ms``: time the host kept a runnable vCPU off a physical CPU).
    What remains is attributable to one init operation and blocks the doctor
    gate. A stage without ``steal_ms`` (an older guest, or a hypervisor that
    reports no steal) is budgeted on wall time as before. Slow stages are
    returned unmodified, so a failure still shows raw duration and steal.
    """
    total_ms = sum(int(stage.get("duration_ms", 0)) for stage in stages)
    slow_stages = tuple(
        stage
        for stage in stages
        if stage.get("name") not in UNBUDGETED_STAGES
        and _guest_work_ms(stage) > MAX_BOOT_STAGE_MS
    )
    return BootTimingAssessment(total_ms=total_ms, slow_stages=slow_stages)

"""Pure generational-image retention decisions shared by the storage controller."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

BOUND_FIELDS = ("maximum_count", "maximum_age_hours", "maximum_total_gib")


def resource_decision(resource: dict[str, Any]) -> str:
    """Render one resource's retention policy for reports."""
    retention = resource["retention"]
    if retention == "cache":
        return "retain-cache"
    if retention == "obsolete":
        return "delete-obsolete"
    if retention == "generational":
        return f"retain-current-and-{int(resource['keep_previous'])}-previous"
    boundary = resource.get("release_boundary")
    if boundary:
        return f"release-{boundary}"
    return "release-" + ",".join(str(value) for value in resource.get("release_boundaries", []))


def superseded_generations(
    generations: list[dict[str, Any]], *, keep: str, keep_previous: int
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Keep the newest requested predecessors and retire the rest."""
    superseded = sorted(
        (row for row in generations if row["ref"] != keep),
        key=lambda row: (row["created"], row["ref"]),
        reverse=True,
    )
    return superseded[:keep_previous], superseded[keep_previous:]


def validate_bounds(name: str, resource: dict[str, Any]) -> None:
    """Require a complete positive triple whenever a resource declares bounds."""
    bounds = tuple(resource.get(field) for field in BOUND_FIELDS)
    if any(value is not None for value in bounds) and any(
        not isinstance(value, int) or isinstance(value, bool) or value <= 0
        for value in bounds
    ):
        raise ValueError(
            f"generational resource {name!r} cache bounds must be positive integers"
        )


def protect_generations(
    retained: list[dict[str, Any]],
    removable: list[dict[str, Any]],
    protected: set[str],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Move receipt-pinned generations out of the deletion cohort."""
    pinned = [row for row in removable if row["ref"] in protected]
    return retained + pinned, [row for row in removable if row["ref"] not in protected]


def cache_violations(
    resource: dict[str, Any],
    survivors: list[dict[str, Any]],
    *,
    keep: str,
    now: datetime | None = None,
) -> tuple[str, ...]:
    """Report bounds pinned generations make impossible to satisfy."""
    violations: list[str] = []
    count = resource.get("maximum_count")
    if count is not None and len(survivors) > count:
        violations.append(f"count {len(survivors)} exceeds {count}")
    maximum_bytes = int(resource.get("maximum_total_gib") or 0) * 1024**3
    survivor_bytes = sum(int(row["size_bytes"]) for row in survivors)
    if maximum_bytes and survivor_bytes > maximum_bytes:
        violations.append(f"bytes {survivor_bytes} exceeds {maximum_bytes}")
    maximum_age = int(resource.get("maximum_age_hours") or 0)
    if maximum_age:
        current = (now or datetime.now(UTC)).timestamp()
        cutoff = current - maximum_age * 3600
        expired = sorted(
            row["ref"]
            for row in survivors
            if row["ref"] != keep
            and datetime.fromisoformat(row["created"].replace("Z", "+00:00")).timestamp() < cutoff
        )
        if expired:
            violations.append("expired protected images: " + ", ".join(expired))
        future = sorted(
            row["ref"]
            for row in survivors
            if row["ref"] != keep
            and datetime.fromisoformat(row["created"].replace("Z", "+00:00")).timestamp()
            > current
        )
        if future:
            violations.append("future-dated protected images: " + ", ".join(future))
    return tuple(violations)

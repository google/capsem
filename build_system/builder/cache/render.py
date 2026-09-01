"""Human-readable cache inventory and prune reports."""

from __future__ import annotations

from .models import CacheInventory, PrunePlan


def bytes_label(value: int) -> str:
    size = float(value)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if size < 1024 or unit == "TiB":
            return f"{size:.1f} {unit}"
        size /= 1024
    raise AssertionError("unreachable")


def inventory_text(report: CacheInventory) -> str:
    lines = [
        f"Cache: {report.root}",
        f"Total: {bytes_label(report.logical_bytes)} logical, "
        f"{bytes_label(report.allocated_bytes)} allocated; free: "
        f"{bytes_label(report.filesystem_free_bytes)}",
    ]
    for stage in report.stages:
        kind = "external" if stage.external else "local"
        lines.append(
            f"  {stage.stage_id}: {bytes_label(stage.logical_bytes)} "
            f"({stage.entry_count} entries, {kind})"
        )
    for entry in report.unclassified:
        lines.append(f"  UNCLASSIFIED {entry.relative_path}: {bytes_label(entry.logical_bytes)}")
    for runtime in report.runtimes:
        state = "available" if runtime.available else f"unavailable: {runtime.error}"
        lines.append(
            f"  runtime/{runtime.runtime_id}: native {bytes_label(runtime.native_bytes)}, "
            f"owned {bytes_label(runtime.owned_bytes)} ({len(runtime.resources)} resources; {state})"
        )
    return "\n".join(lines)


def plan_text(plan: PrunePlan, *, preview: bool) -> str:
    heading = "PREVIEW" if preview else "APPLIED"
    lines = [f"{heading}: reclaim {bytes_label(plan.reclaim_bytes)} in {len(plan.actions)} entries"]
    lines.extend(f"  {action.stage_id}/{action.key}: {action.reason}" for action in plan.actions)
    lines.extend(f"  VIOLATION: {violation}" for violation in plan.violations)
    return "\n".join(lines)

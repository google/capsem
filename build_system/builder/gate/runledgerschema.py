"""Typed durable rows distilled from complete gate run journals."""

from __future__ import annotations

from .configschema import Strict
from .runlogschema import OK


class StepRow(Strict):
    """One measured step, excluding skipped and carried work from trends."""

    duration_ms: float
    status: str
    resource_ms: float = 0.0
    dependency_ms: float = 0.0


class LedgerRow(Strict):
    """One finished run, distilled to what longitudinal analysis needs."""

    row_schema: str
    run_id: str
    command: str
    head: str
    status: str
    total_ms: float
    identity: str
    critical_path: tuple[str, ...]
    steps: dict[str, StepRow]

    def measured(self, label: str) -> float | None:
        """This step's duration, or nothing when it did not perform work."""
        row = self.steps.get(label)
        return row.duration_ms if row is not None and row.status == OK else None

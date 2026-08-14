"""Namespacing a fragment's steps into a shared plan.

Split from `plan`, which owns the graph. This owns nothing: it is a view that
prefixes labels on the way in and records which stage they came from, so the
plan stays the single holder of nodes and edges.

The seam is worth having beyond the line count. Composed into one plan,
`test-static` and `test-functional` both want a step called `sign`, and both
legitimately -- the binaries are signed after the coverage build and again
before the VM suites. Namespacing is what lets a fragment be written without
knowing who else is in the plan.
"""

from __future__ import annotations

from dataclasses import replace
from typing import TYPE_CHECKING

from .execution import Requires, Step

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .plan import Plan


class Phase:
    """One fragment's steps, added to a shared plan under one namespace.

    Composed into a single plan, `test-static` and `test-functional` both want
    a step called `sign`, and both legitimately -- the binaries are signed after
    the coverage build and again before the VM suites. Namespacing makes them
    `static.sign` and `functional.sign`, which is also what the run log and the
    timing report then say, so a slow step names the phase it belongs to.
    """

    def __init__(self, plan: Plan, prefix: str) -> None:
        self._plan = plan
        self._prefix = prefix

    def add(
        self,
        step: Step,
        *,
        after: tuple[Step, ...] = (),
        requires: Requires = Requires.UNDECLARED,
    ) -> Step:
        added = self._plan.add(
            replace(step, label=self.label(step.label)), after=after, requires=requires
        )
        self._plan.record_stage(added.label, self._prefix)
        return added

    def shared(
        self,
        step: Step,
        *,
        after: tuple[Step, ...] = (),
        requires: Requires = Requires.UNDECLARED,
    ) -> Step:
        """Groundwork several phases need, kept out of any one namespace."""
        return self._plan.shared(step, after=after, requires=requires)

    def label(self, name: str) -> str:
        return f"{self._prefix}.{name}"

"""Showing a plan to a person: the dry run, and the diagram.

Separate from `plan`, which runs one. Rendering and executing are different
concerns, and only one of them may touch the machine -- a dry run with side
effects is not a dry run.
"""

from __future__ import annotations

from operator import attrgetter
from typing import TYPE_CHECKING

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .plan import Plan


def describe(plan: Plan) -> str:
    """The dry run: what would run, in what order, and what it invokes."""
    waves = plan.order()
    actions = sum(len(step.actions) for step in plan._steps)
    lines = [
        f"plan: {plan.name} -- {len(plan._steps)} steps, {actions} actions, {len(waves)} waves",
        "",
    ]
    for position, wave in enumerate(waves, start=1):
        for offset, step in enumerate(sorted(wave, key=attrgetter("label"))):
            held = (
                "  [" + ", ".join(sorted(e.name for e in step.contends)) + "]"
                if step.contends
                else ""
            )
            # The wave number once, on its first step: everything under it
            # runs at the same time, and repeating the number says the
            # opposite to anyone skimming.
            marker = f"{position:>3}" if offset == 0 else "   "
            lines.append(f"  {marker}  {step.label}{held}")
            lines += [f"          {rendering}" for rendering in step.render()]
    lines += ["", "nothing was executed (--dry-run)"]
    return "\n".join(lines)


def mermaid(plan: Plan) -> str:
    """The graph, for a bug report or the documentation site."""
    lines = ["graph TD"]
    for step in plan._steps:
        lines.append(f"  {_node(step.label)}[{step.label}]")
    lines += [f"  {_node(before)} --> {_node(after)}" for before, after in plan.edges]
    return "\n".join(lines)


def _node(label: str) -> str:
    """A mermaid-safe identifier for a step label."""
    return "".join(character if character.isalnum() else "_" for character in label)

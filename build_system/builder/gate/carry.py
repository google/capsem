"""Validate ephemeral products before trusting a carried step."""

from __future__ import annotations

from typing import TYPE_CHECKING

from .errors import GateError
from .execution import Requires

if TYPE_CHECKING:
    from .context import Context
    from .plan import Plan


def validate(plan: Plan, context: Context) -> None:
    """Refuse continuation before work when a carried product disappeared."""
    for label in plan.labels:
        if label not in context.carried:
            continue
        step = plan.step_named(label)
        consumers = {
            after
            for before, after in plan.edges
            if before == label and plan.requires_of(before, after) is Requires.ARTIFACT
        }
        # Most checks protect durable evidence and remain fail-closed.  A
        # producer with explicit artifact consumers instead protects an
        # intermediate whose lifetime ends once those consumers complete.
        # Bounded storage may legitimately reclaim it after that point; later
        # ordering descendants do not make its bytes live again.
        if consumers and consumers <= context.carried:
            continue
        for check in step.carry_checks:
            try:
                with context.journal.action(check):
                    check.perform(context)
            except Exception as error:
                raise GateError(
                    f"cannot carry {label!r}: {error}; resume with --from {label} "
                    "so its owning materializer runs again"
                ) from error

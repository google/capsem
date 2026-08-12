"""Validate ephemeral products before trusting a carried step."""

from __future__ import annotations

from typing import TYPE_CHECKING

from .errors import GateError

if TYPE_CHECKING:
    from .context import Context
    from .plan import Plan


def validate(plan: Plan, context: Context) -> None:
    """Refuse continuation before work when a carried product disappeared."""
    for label in plan.labels:
        if label not in context.carried:
            continue
        step = plan.step_named(label)
        for check in step.carry_checks:
            try:
                with context.journal.action(check):
                    check.perform(context)
            except Exception as error:
                raise GateError(
                    f"cannot carry {label!r}: {error}; resume with --from {label} "
                    "so its owning materializer runs again"
                ) from error

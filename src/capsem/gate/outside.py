"""Run one declared action through the authenticated pre-sandbox capability."""

from __future__ import annotations

from dataclasses import replace

from .actions import Action
from .context import Context
from .scopeenv import action_environment


class Outside(Action, name="outside-sandbox"):
    """Move one materializer, never its consumers, outside the sandbox."""

    def __init__(self, action: Action) -> None:
        self._action = action

    def render(self) -> str:
        return f"{self._action.render()} [outside kernel sandbox]"

    def perform(self, context: Context) -> None:
        self._action.perform(
            replace(
                context,
                runner=context.external_runner,
                outside_runner=None,
                env=action_environment(
                    context.config,
                    context.env,
                    {},
                    outside_sandbox=True,
                ),
            )
        )

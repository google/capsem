"""The run-time question a phase asks before it boots staged or built content.

A plan may not depend on build output, so whether the runtime content a phase
points its VMs at is complete cannot be decided while the plan is built. It is
a step that runs, after whatever produced the content.
"""

from __future__ import annotations

from .actions import Action
from .content import RuntimeContent
from .context import Context


class ContentComplete(Action, name="content-complete"):
    """Refuse unless the exact pair of assets and configuration is complete."""

    def __init__(self, content: RuntimeContent) -> None:
        self._content = content

    def render(self) -> str:
        return f"check the runtime content in {self._content.root} is complete"

    def perform(self, context: Context) -> None:
        # Only this host's architecture boots here; a lane qualifying one
        # architecture must not be refused for the other's absence.
        self._content.require_complete(context.config, arches=(context.config.host_arch(),))

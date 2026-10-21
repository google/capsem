"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool

from .model_base import Model
from .terminal_event_kind import TerminalEventKind
from .terminal_notification import TerminalNotification


class TerminalEvent(Model):
    kind: TerminalEventKind
    notification: TerminalNotification | None = None
    visual: StrictBool

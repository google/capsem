"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model
from .terminal_cursor import TerminalCursor
from .terminal_event import TerminalEvent
from .terminal_modes import TerminalModes
from .terminal_notification import TerminalNotification


class TerminalState(Model):
    bell_count: Annotated[StrictInt, Field(ge=0)]
    cols: Annotated[StrictInt, Field(ge=0)]
    cursor: TerminalCursor
    events: list[TerminalEvent]
    icon_name: StrictStr
    last_notification: TerminalNotification | None = None
    modes: TerminalModes
    reset: StrictBool
    revision: Annotated[StrictInt, Field(ge=0)]
    rows: Annotated[StrictInt, Field(ge=0)]
    title: StrictStr

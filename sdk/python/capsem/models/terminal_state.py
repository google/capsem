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
    cols: Annotated[Annotated[StrictInt, Field(ge=1)], Field(le=512)]
    cursor: TerminalCursor
    events: Annotated[list[TerminalEvent], Field(max_length=32)]
    icon_name: Annotated[StrictStr, Field(max_length=1024)]
    last_notification: TerminalNotification | None = None
    modes: TerminalModes
    reset: StrictBool
    revision: Annotated[StrictInt, Field(ge=0)]
    rows: Annotated[Annotated[StrictInt, Field(ge=1)], Field(le=256)]
    title: Annotated[StrictStr, Field(max_length=1024)]

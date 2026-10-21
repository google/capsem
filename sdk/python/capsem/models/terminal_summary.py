"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .terminal_notification import TerminalNotification


class TerminalSummary(Model):
    bell_count: Annotated[StrictInt, Field(ge=0)]
    last_notification: TerminalNotification | None = None
    revision: Annotated[StrictInt, Field(ge=0)]
    title: Annotated[StrictStr, Field(max_length=1024)]

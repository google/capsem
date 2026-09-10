"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt

from .history_entry import HistoryEntry
from .model_base import Model


class HistoryResponse(Model):
    commands: list[HistoryEntry]
    has_more: StrictBool
    total: Annotated[StrictInt, Field(ge=0)]

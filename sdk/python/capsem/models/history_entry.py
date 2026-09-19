"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .history_details import HistoryDetails
from .history_layer import HistoryLayer
from .model_base import Model


class HistoryEntry(Model):
    command: StrictStr
    details: HistoryDetails
    duration_ms: Annotated[StrictInt, Field(ge=0)] | None = None
    exit_code: StrictInt | None = None
    layer: HistoryLayer
    stderr_preview: StrictStr | None = None
    stdout_preview: StrictStr | None = None
    timestamp: StrictStr

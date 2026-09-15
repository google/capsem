"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .file_event_action import FileEventAction
from .model_base import Model


class FileEvent(Model):
    action: FileEventAction
    credential_ref: StrictStr | None = None
    event_id: StrictStr
    path: StrictStr
    size: Annotated[StrictInt, Field(ge=0)] | None = None
    timestamp: StrictStr
    trace_id: StrictStr | None = None

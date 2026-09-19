"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .file_event_action import FileEventAction
from .model_base import Model


class FileActionCount(Model):
    action: FileEventAction
    count: Annotated[StrictInt, Field(ge=0)]

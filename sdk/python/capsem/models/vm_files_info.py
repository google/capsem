"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .file_action_count import FileActionCount
from .model_base import Model


class VmFilesInfo(Model):
    actions: list[FileActionCount]
    total_events: Annotated[StrictInt, Field(ge=0)]

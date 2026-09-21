"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .file_change import FileChange
from .model_base import Model


class ChangesResponse(Model):
    changes: list[FileChange]
    checkpoint: StrictStr
    has_more: StrictBool
    total: Annotated[StrictInt, Field(ge=0)]

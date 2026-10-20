"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .asset_file_state import AssetFileState
from .model_base import Model


class AssetFileStatus(Model):
    actual_size: Annotated[StrictInt, Field(ge=0)] | None = None
    expected_hash: StrictStr
    expected_size: Annotated[StrictInt, Field(ge=0)] | None = None
    kind: StrictStr
    name: StrictStr
    path: StrictStr
    status: AssetFileState

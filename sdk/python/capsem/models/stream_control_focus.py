"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated, Literal

from pydantic import Field, StrictInt

from .model_base import Model


class StreamControlFocus(Model):
    cols: Annotated[Annotated[StrictInt, Field(ge=1)], Field(le=512)]
    rows: Annotated[Annotated[StrictInt, Field(ge=1)], Field(le=256)]
    type: Literal["focus"]

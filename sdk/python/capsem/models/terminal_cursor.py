"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt

from .model_base import Model


class TerminalCursor(Model):
    col: Annotated[StrictInt, Field(ge=0)]
    hidden: StrictBool
    row: Annotated[StrictInt, Field(ge=0)]

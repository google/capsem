"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictStr

from .model_base import Model


class TerminalNotification(Model):
    body: Annotated[StrictStr, Field(max_length=1024)]
    id: Annotated[StrictStr, Field(max_length=1024)]
    title: Annotated[StrictStr, Field(max_length=1024)]

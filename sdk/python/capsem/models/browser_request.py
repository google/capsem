"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class BrowserRequest(Model):
    id: Annotated[StrictInt, Field(ge=0)]
    owner_generation: StrictStr | None = None
    url: Annotated[StrictStr, Field(max_length=8192)]

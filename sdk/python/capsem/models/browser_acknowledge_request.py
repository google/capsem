"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt, StrictStr

from .model_base import Model


class BrowserAcknowledgeRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    owner_generation: StrictStr
    request_id: Annotated[StrictInt, Field(ge=0)]

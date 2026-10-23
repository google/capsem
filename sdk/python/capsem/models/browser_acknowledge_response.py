"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class BrowserAcknowledgeResponse(Model):
    owner_generation: StrictStr
    request_id: Annotated[StrictInt, Field(ge=0)]
    vm_id: StrictStr

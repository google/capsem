"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .model_base import Model


class VmNetworkInfo(Model):
    allowed_requests: Annotated[StrictInt, Field(ge=0)]
    bytes_received: Annotated[StrictInt, Field(ge=0)]
    bytes_sent: Annotated[StrictInt, Field(ge=0)]
    denied_requests: Annotated[StrictInt, Field(ge=0)]
    errors: Annotated[StrictInt, Field(ge=0)]
    total_requests: Annotated[StrictInt, Field(ge=0)]

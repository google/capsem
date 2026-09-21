"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model


class McpRefreshResponse(Model):
    instances: Annotated[StrictInt, Field(ge=0)]
    server_id: StrictStr
    success: StrictBool

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model


class McpServerInfoResponse(Model):
    custom_header_count: Annotated[StrictInt, Field(ge=0)]
    enabled: StrictBool
    has_auth_credential: StrictBool
    is_stdio: StrictBool
    name: StrictStr
    running: StrictBool
    source: StrictStr
    tool_count: Annotated[StrictInt, Field(ge=0)]
    url: StrictStr

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class McpUsage(Model):
    bytes_received: Annotated[StrictInt, Field(ge=0)]
    bytes_sent: Annotated[StrictInt, Field(ge=0)]
    call_count: Annotated[StrictInt, Field(ge=0)]
    duration_ms: Annotated[StrictInt, Field(ge=0)]
    server_name: StrictStr | None = None
    tool_name: StrictStr

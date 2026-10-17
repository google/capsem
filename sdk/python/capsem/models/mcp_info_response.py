"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt

from .model_base import Model


class McpInfoResponse(Model):
    builtin_local_enabled: StrictBool
    manual_server_count: Annotated[StrictInt, Field(ge=0)]
    server_count: Annotated[StrictInt, Field(ge=0)]

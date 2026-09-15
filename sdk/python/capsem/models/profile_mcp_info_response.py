"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model


class ProfileMcpInfoResponse(Model):
    builtin_local_enabled: StrictBool
    manual_server_count: Annotated[StrictInt, Field(ge=0)]
    profile_id: StrictStr
    server_count: Annotated[StrictInt, Field(ge=0)]

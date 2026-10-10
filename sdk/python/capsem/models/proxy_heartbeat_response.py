"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class ProxyHeartbeatResponse(Model):
    lease_expires_unix_ms: Annotated[StrictInt, Field(ge=0)]
    session_id: StrictStr

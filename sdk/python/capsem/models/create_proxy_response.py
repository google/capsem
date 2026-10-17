"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class CreateProxyResponse(Model):
    base_url: StrictStr
    bind: StrictStr
    lease_expires_unix_ms: Annotated[StrictInt, Field(ge=0)]
    lease_token: StrictStr
    port: Annotated[StrictInt, Field(ge=0)]
    provider: StrictStr
    session_id: StrictStr

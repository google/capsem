"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt, StrictStr

from .model_base import Model


class ManagedSessionOwnership(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    expires_at: StrictStr
    generation: StrictStr
    lease_seconds: Annotated[StrictInt, Field(ge=0)]
    renew_after_seconds: Annotated[StrictInt, Field(ge=0)]
    request_id: StrictStr

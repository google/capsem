"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model


class ProvisionRequest(Model):
    nonnullable_optional = frozenset(['persistent'])
    cpus: Annotated[StrictInt, Field(ge=0)] | None = None
    env: dict[str, StrictStr] | None = None
    from_: StrictStr | None = Field(default=None, alias='from')
    name: StrictStr | None = None
    persistent: StrictBool | None = None
    profile_id: StrictStr
    ram_mb: Annotated[StrictInt, Field(ge=0)] | None = None

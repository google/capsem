"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .model_base import Model


class PurgeResponse(Model):
    ephemeral_purged: Annotated[StrictInt, Field(ge=0)]
    persistent_purged: Annotated[StrictInt, Field(ge=0)]
    purged: Annotated[StrictInt, Field(ge=0)]

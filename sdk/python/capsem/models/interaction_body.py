"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model


class InteractionBody(Model):
    body_hash: StrictStr
    content_type: StrictStr | None = None
    direction: StrictStr
    event_id: StrictStr
    original_bytes: Annotated[StrictInt, Field(ge=0)]
    source_table: StrictStr
    stored_bytes: Annotated[StrictInt, Field(ge=0)]
    truncated: StrictBool

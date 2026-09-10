"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .body_direction import BodyDirection
from .model_base import Model


class EventBody(Model):
    body: StrictStr
    body_hash: StrictStr
    content_type: StrictStr | None = None
    direction: BodyDirection
    event_id: StrictStr
    original_bytes: Annotated[StrictInt, Field(ge=0)]
    stored_bytes: Annotated[StrictInt, Field(ge=0)]
    truncated: StrictBool

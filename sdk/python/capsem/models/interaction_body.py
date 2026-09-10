"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .body_direction import BodyDirection
from .captured_payload import CapturedPayload
from .model_base import Model


class InteractionBody(Model):
    body_hash: StrictStr
    content_type: StrictStr | None = None
    direction: BodyDirection
    event_id: StrictStr
    original_bytes: Annotated[StrictInt, Field(ge=0)]
    payload: CapturedPayload
    stored_bytes: Annotated[StrictInt, Field(ge=0)]

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .body_encoding import BodyEncoding
from .model_base import Model


class ArchivedEventBody(Model):
    body_hash: StrictStr
    content: StrictStr
    content_type: StrictStr | None = None
    direction: StrictStr
    encoding: BodyEncoding
    event_id: StrictStr
    original_bytes: Annotated[StrictInt, Field(ge=0)]
    source_table: StrictStr
    stored_bytes: Annotated[StrictInt, Field(ge=0)]
    truncated: StrictBool
    truncated_for_transport: StrictBool

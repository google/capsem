"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .interaction_content import InteractionContent
from .model_base import Model


class Interaction(Model):
    content: InteractionContent
    event_id: StrictStr
    item_index: Annotated[StrictInt, Field(ge=0)] | None = None
    model_call_id: StrictInt | None = None
    model_event_id: StrictStr | None = None
    timestamp: StrictStr
    trace_id: StrictStr | None = None
    turn_id: StrictStr | None = None

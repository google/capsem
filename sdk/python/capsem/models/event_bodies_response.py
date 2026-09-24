"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .archived_event_body import ArchivedEventBody
from .model_base import Model


class EventBodiesResponse(Model):
    bodies: list[ArchivedEventBody]
    event_id: StrictStr

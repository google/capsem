"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .model_base import Model
from .timeline_event import TimelineEvent


class TimelineResponse(Model):
    events: list[TimelineEvent]

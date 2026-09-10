"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .timeline_layer import TimelineLayer
from .timeline_reference import TimelineReference
from .timeline_status import TimelineStatus


class TimelineEvent(Model):
    duration_ms: Annotated[StrictInt, Field(ge=0)] | None = None
    layer: TimelineLayer
    ref: TimelineReference
    status: TimelineStatus | None = None
    summary: StrictStr
    timestamp: StrictStr
    trace_id: StrictStr | None = None

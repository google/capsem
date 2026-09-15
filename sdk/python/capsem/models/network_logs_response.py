"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model
from .network_log_event import NetworkLogEvent


class NetworkLogsResponse(Model):
    cursor: StrictStr
    events: list[NetworkLogEvent]
    next_cursor: StrictStr | None = None

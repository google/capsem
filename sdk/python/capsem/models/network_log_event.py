"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictInt, StrictStr

from .model_base import JsonValue, Model


class NetworkLogEvent(Model):
    connection_id: StrictStr | None = None
    event: JsonValue
    event_id: StrictStr
    event_type: StrictStr
    sequence: StrictInt
    timestamp_unix_ms: StrictInt

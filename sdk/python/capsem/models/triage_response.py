"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .host_triage_response import HostTriageResponse
from .model_base import JsonValue, Model


class TriageResponse(Model):
    host: HostTriageResponse
    rank: list[StrictStr]
    session: JsonValue
    session_id: StrictStr | None = None
    since: StrictStr

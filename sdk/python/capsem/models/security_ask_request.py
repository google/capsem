"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictInt, StrictStr

from .model_base import Model
from .security_ask_status import SecurityAskStatus


class SecurityAskRequest(Model):
    ask_id: StrictStr
    created_at_unix_ms: StrictInt
    event_id: StrictStr
    event_type: StrictStr
    expires_at_unix_ms: StrictInt
    resolution_reason: StrictStr | None = None
    resolved_at_unix_ms: StrictInt | None = None
    rule_id: StrictStr
    rule_name: StrictStr
    status: SecurityAskStatus
    summary: StrictStr

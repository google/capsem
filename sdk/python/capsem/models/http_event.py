"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .network_decision import NetworkDecision


class HttpEvent(Model):
    bytes_received: Annotated[StrictInt, Field(ge=0)] | None = None
    bytes_sent: Annotated[StrictInt, Field(ge=0)] | None = None
    credential_ref: StrictStr | None = None
    decision: NetworkDecision
    domain: StrictStr
    duration_ms: Annotated[StrictInt, Field(ge=0)] | None = None
    event_id: StrictStr
    matched_rule: StrictStr | None = None
    method: StrictStr | None = None
    path: StrictStr | None = None
    policy_rule: StrictStr | None = None
    port: Annotated[StrictInt, Field(ge=0)] | None = None
    query: StrictStr | None = None
    request_headers: StrictStr | None = None
    response_headers: StrictStr | None = None
    status_code: Annotated[StrictInt, Field(ge=0)] | None = None
    timestamp: StrictStr
    trace_id: StrictStr | None = None

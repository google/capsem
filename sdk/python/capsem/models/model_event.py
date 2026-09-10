"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class ModelEvent(Model):
    credential_ref: StrictStr | None = None
    duration_ms: Annotated[StrictInt, Field(ge=0)] | None = None
    event_id: StrictStr
    input_tokens: Annotated[StrictInt, Field(ge=0)] | None = None
    method: StrictStr
    model: StrictStr | None = None
    output_tokens: Annotated[StrictInt, Field(ge=0)] | None = None
    path: StrictStr
    provider: StrictStr
    response_bytes: Annotated[StrictInt, Field(ge=0)] | None = None
    status_code: Annotated[StrictInt, Field(ge=0)] | None = None
    stop_reason: StrictStr | None = None
    timestamp: StrictStr
    trace_id: StrictStr | None = None

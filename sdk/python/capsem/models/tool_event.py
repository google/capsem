"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model
from .tool_decision import ToolDecision
from .tool_origin import ToolOrigin


class ToolEvent(Model):
    arguments: StrictStr | None = None
    bytes: Annotated[StrictInt, Field(ge=0)]
    call_id: StrictStr
    credential_ref: StrictStr | None = None
    decision: ToolDecision
    duration_ms: Annotated[StrictInt, Field(ge=0)]
    error_message: StrictStr | None = None
    event_id: StrictStr
    method: StrictStr | None = None
    model_call_id: StrictInt | None = None
    model_parent_missing: StrictBool
    process_name: StrictStr | None = None
    response_preview: StrictStr | None = None
    server_name: StrictStr
    source: ToolOrigin
    timestamp: StrictStr | None = None
    tool_name: StrictStr

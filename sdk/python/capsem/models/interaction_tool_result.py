"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .captured_payload import CapturedPayload
from .interaction_tool_result_kind import InteractionToolResultKind
from .model_base import Model


class InteractionToolResult(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    call_id: StrictStr
    error_message: StrictStr | None = None
    is_error: StrictBool | None = None
    kind: InteractionToolResultKind
    payload: CapturedPayload | None = None
    response: CapturedPayload | None = None

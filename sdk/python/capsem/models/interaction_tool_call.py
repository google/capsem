"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .captured_payload import CapturedPayload
from .interaction_tool_call_kind import InteractionToolCallKind
from .interaction_tool_result import InteractionToolResult
from .model_base import Model
from .tool_decision import ToolDecision
from .tool_origin import ToolOrigin


class InteractionToolCall(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    arguments: CapturedPayload | None = None
    call_id: StrictStr
    decision: ToolDecision
    kind: InteractionToolCallKind
    origin: ToolOrigin
    request: CapturedPayload | None = None
    result: InteractionToolResult | None = None
    server_name: StrictStr | None = None
    tool_name: StrictStr

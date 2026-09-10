"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .interaction_message import InteractionMessage
from .interaction_request import InteractionRequest
from .interaction_tool_call import InteractionToolCall
from .interaction_tool_result import InteractionToolResult

InteractionContent: TypeAlias = InteractionRequest | InteractionMessage | InteractionToolCall | InteractionToolResult

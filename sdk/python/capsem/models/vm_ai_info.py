"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictFloat, StrictInt

from .mcp_usage import McpUsage
from .model_base import Model
from .model_usage import ModelUsage


class VmAiInfo(Model):
    mcp: list[McpUsage]
    model_call_count: Annotated[StrictInt, Field(ge=0)]
    models: list[ModelUsage]
    total_estimated_cost_usd: StrictFloat
    total_input_tokens: Annotated[StrictInt, Field(ge=0)]
    total_output_tokens: Annotated[StrictInt, Field(ge=0)]
    total_thinking_tokens: Annotated[StrictInt, Field(ge=0)]
    total_tool_calls: Annotated[StrictInt, Field(ge=0)]

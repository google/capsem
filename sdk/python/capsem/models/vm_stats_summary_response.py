"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictFloat, StrictInt

from .model_base import Model


class VmStatsSummaryResponse(Model):
    allowed_requests: Annotated[StrictInt, Field(ge=0)]
    denied_requests: Annotated[StrictInt, Field(ge=0)]
    total_estimated_cost: StrictFloat
    total_input_tokens: Annotated[StrictInt, Field(ge=0)]
    total_output_tokens: Annotated[StrictInt, Field(ge=0)]
    total_requests: Annotated[StrictInt, Field(ge=0)]
    total_thinking_tokens: Annotated[StrictInt, Field(ge=0)]
    total_tool_calls: Annotated[StrictInt, Field(ge=0)]

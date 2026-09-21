"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictFloat, StrictInt, StrictStr

from .model_base import Model


class ModelUsage(Model):
    call_count: Annotated[StrictInt, Field(ge=0)]
    duration_ms: Annotated[StrictInt, Field(ge=0)]
    estimated_cost_usd: StrictFloat
    input_tokens: Annotated[StrictInt, Field(ge=0)]
    model: StrictStr
    output_tokens: Annotated[StrictInt, Field(ge=0)]
    provider: StrictStr

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt, StrictStr

from .agent_access_scope import AgentAccessScope
from .model_base import Model, UuidString


class CreateAgentAccessRequest(Model):
    private_input = True
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid", hide_input_in_errors=True)
    lease_seconds: Annotated[StrictInt, Field(ge=0)]
    ownership_token: Annotated[StrictStr, Field(pattern='^[0-9a-f]{64}$', min_length=64, max_length=64)] = Field(repr=False)
    request_id: UuidString
    scope: AgentAccessScope

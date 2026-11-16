"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .agent_access_summary import AgentAccessSummary
from .model_base import Model


class CreateAgentAccessResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    access: AgentAccessSummary

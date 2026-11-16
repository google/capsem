"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .agent_access_scope import AgentAccessScope
from .model_base import Model


class AgentAccessSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    expires_at: StrictStr
    id: StrictStr
    scope: AgentAccessScope

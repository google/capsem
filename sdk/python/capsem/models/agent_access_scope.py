"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .agent_operation import AgentOperation
from .agent_session_scope import AgentSessionScope
from .model_base import Model


class AgentAccessScope(Model):
    environment_ids: list[StrictStr]
    operations: list[AgentOperation]
    sessions: list[AgentSessionScope]

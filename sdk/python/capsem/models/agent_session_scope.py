"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class AgentSessionScope(Model):
    owner_generation: StrictStr
    session_id: StrictStr

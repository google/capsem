"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model
from .security_ask_decision import SecurityAskDecision


class SecurityAskResolveRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    decision: SecurityAskDecision
    generation: StrictStr

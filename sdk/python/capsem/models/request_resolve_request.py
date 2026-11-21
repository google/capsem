"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model
from .request_decision import RequestDecision


class RequestResolveRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    decision: RequestDecision
    observed_revision: StrictStr
    reason: StrictStr

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictInt, StrictStr

from .model_base import Model


class RequestDecisionDetails(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    actor: StrictStr
    reason: StrictStr
    timestamp_unix_ms: StrictInt

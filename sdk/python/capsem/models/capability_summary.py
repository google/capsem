"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .model_base import Model


class CapabilitySummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    consumer: StrictStr
    id: StrictStr
    label: StrictStr
    required_scopes: list[StrictStr]
    supported: StrictBool

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .model_base import Model


class McpCatalogRequirement(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    description: StrictStr
    required: StrictBool

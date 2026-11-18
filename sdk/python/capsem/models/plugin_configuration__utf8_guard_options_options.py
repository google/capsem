"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool

from .model_base import Model


class PluginConfiguration_Utf8GuardOptionsOptions(Model):
    nonnullable_optional = frozenset(['inbound', 'outbound'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    inbound: StrictBool | None = None
    outbound: StrictBool | None = None

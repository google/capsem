"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .model_base import Model
from .plugin_configuration__pii_filter_options import (
    PluginConfiguration_PiiFilterOptions,
)


class PiiFilterConfiguration(Model):
    nonnullable_optional = frozenset(['inbound', 'outbound'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    inbound: PluginConfiguration_PiiFilterOptions | None = None
    outbound: PluginConfiguration_PiiFilterOptions | None = None

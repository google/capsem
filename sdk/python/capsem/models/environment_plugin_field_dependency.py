"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_plugin_field_value import EnvironmentPluginFieldValue
from .model_base import Model


class EnvironmentPluginFieldDependency(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    field: StrictStr
    not_equals: EnvironmentPluginFieldValue

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_plugin_field_dependency import EnvironmentPluginFieldDependency
from .environment_plugin_field_value import EnvironmentPluginFieldValue
from .model_base import Model
from .widget import Widget


class EnvironmentPluginField(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    category: StrictStr
    choices: list[StrictStr]
    default_value: EnvironmentPluginFieldValue
    dependencies: list[EnvironmentPluginFieldDependency]
    description: StrictStr
    id: StrictStr
    name: StrictStr
    widget: Widget | None = None

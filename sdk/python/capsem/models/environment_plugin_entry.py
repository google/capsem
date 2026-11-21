"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool

from .environment_plugin_descriptor import EnvironmentPluginDescriptor
from .environment_plugin_direction_state import EnvironmentPluginDirectionState
from .environment_plugin_field_value import EnvironmentPluginFieldValue
from .environment_plugin_source import EnvironmentPluginSource
from .model_base import Model


class EnvironmentPluginEntry(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    configured: dict[str, EnvironmentPluginFieldValue]
    descriptor: EnvironmentPluginDescriptor
    directions: list[EnvironmentPluginDirectionState]
    effective: dict[str, EnvironmentPluginFieldValue]
    locked: StrictBool
    source: EnvironmentPluginSource

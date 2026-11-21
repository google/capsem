"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictBool, StrictInt, StrictStr

from .environment_plugin_field import EnvironmentPluginField
from .model_base import Model
from .plugin_control_group import PluginControlGroup
from .plugin_detail_capability import PluginDetailCapability
from .plugin_direction import PluginDirection
from .plugin_event_family import PluginEventFamily
from .plugin_id import PluginId
from .plugin_stage import PluginStage
from .security_plugin_mode import SecurityPluginMode


class EnvironmentPluginDescriptor(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    description: StrictStr
    detail_capabilities: list[PluginDetailCapability]
    diagnostic: StrictBool
    directions: list[PluginDirection]
    event_families: list[PluginEventFamily]
    fields: list[EnvironmentPluginField]
    groups: list[PluginControlGroup]
    id: PluginId
    limitations: list[StrictStr]
    modes: list[SecurityPluginMode]
    name: StrictStr
    required: StrictBool
    stage: PluginStage
    version: Annotated[StrictInt, Field(ge=0)]

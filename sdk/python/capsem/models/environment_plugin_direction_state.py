"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .detection_level import DetectionLevel
from .model_base import Model
from .plugin_direction import PluginDirection
from .security_plugin_mode import SecurityPluginMode


class EnvironmentPluginDirectionState(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    detection_level: DetectionLevel | None
    direction: PluginDirection
    mode: SecurityPluginMode

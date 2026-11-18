"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .detection_level import DetectionLevel
from .model_base import Model
from .plugin_configuration__utf8_guard_options_mode import (
    PluginConfiguration_Utf8GuardOptionsMode,
)
from .plugin_configuration__utf8_guard_options_options import (
    PluginConfiguration_Utf8GuardOptionsOptions,
)


class PluginConfiguration_Utf8GuardOptions(Model):
    nonnullable_optional = frozenset(['detection_level', 'mode', 'options'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    detection_level: DetectionLevel | None = None
    mode: PluginConfiguration_Utf8GuardOptionsMode | None = None
    options: PluginConfiguration_Utf8GuardOptionsOptions | None = None

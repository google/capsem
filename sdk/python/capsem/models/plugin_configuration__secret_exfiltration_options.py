"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .detection_level import DetectionLevel
from .model_base import Model
from .plugin_configuration__secret_exfiltration_options_mode import (
    PluginConfiguration_SecretExfiltrationOptionsMode,
)
from .plugin_configuration__secret_exfiltration_options_options import (
    PluginConfiguration_SecretExfiltrationOptionsOptions,
)


class PluginConfiguration_SecretExfiltrationOptions(Model):
    nonnullable_optional = frozenset(['detection_level', 'mode', 'options'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    detection_level: DetectionLevel | None = None
    mode: PluginConfiguration_SecretExfiltrationOptionsMode | None = None
    options: PluginConfiguration_SecretExfiltrationOptionsOptions | None = None

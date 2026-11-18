"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .detection_level import DetectionLevel
from .model_base import Model
from .plugin_configuration__dummy_pre_eicar_options_mode import (
    PluginConfiguration_DummyPreEicarOptionsMode,
)
from .plugin_configuration__dummy_pre_eicar_options_options import (
    PluginConfiguration_DummyPreEicarOptionsOptions,
)


class PluginConfiguration_DummyPreEicarOptions(Model):
    nonnullable_optional = frozenset(['detection_level', 'mode', 'options'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    detection_level: DetectionLevel | None = None
    mode: PluginConfiguration_DummyPreEicarOptionsMode | None = None
    options: PluginConfiguration_DummyPreEicarOptionsOptions | None = None

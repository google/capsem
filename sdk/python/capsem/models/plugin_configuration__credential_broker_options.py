"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import ConfigDict

from .detection_level import DetectionLevel
from .model_base import Model
from .plugin_configuration__credential_broker_options_options import (
    PluginConfiguration_CredentialBrokerOptionsOptions,
)


class PluginConfiguration_CredentialBrokerOptions(Model):
    nonnullable_optional = frozenset(['detection_level', 'mode', 'options'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    detection_level: DetectionLevel | None = None
    mode: Literal["rewrite"] | None = None
    options: PluginConfiguration_CredentialBrokerOptionsOptions | None = None

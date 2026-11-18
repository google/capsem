"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .model_base import Model


class PluginConfiguration_LogSanitizerOptionsOptions(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")

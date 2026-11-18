"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool

from .model_base import Model


class PluginConfiguration_CredentialBrokerOptionsOptions(Model):
    nonnullable_optional = frozenset(['capture_enabled'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    capture_enabled: StrictBool | None = None

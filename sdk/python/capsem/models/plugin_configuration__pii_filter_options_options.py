"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool

from .model_base import Model


class PluginConfiguration_PiiFilterOptionsOptions(Model):
    nonnullable_optional = frozenset(['card', 'email', 'phone', 'ssn'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    card: StrictBool | None = None
    email: StrictBool | None = None
    phone: StrictBool | None = None
    ssn: StrictBool | None = None

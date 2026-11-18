"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .model_base import Model


class EnvironmentPackages(Model):
    nonnullable_optional = frozenset(['allows_package_manager', 'registries'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    allows_package_manager: StrictBool | None = None
    registries: list[StrictStr] | None = None

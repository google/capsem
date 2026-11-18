"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_catalog_selections__environment_catalog_pin_selected_item import (
    EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItem,
)
from .model_base import Model


class EnvironmentCatalogSelections_EnvironmentCatalogPin(Model):
    nonnullable_optional = frozenset(['selected'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    selected: list[EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItem] | None = None

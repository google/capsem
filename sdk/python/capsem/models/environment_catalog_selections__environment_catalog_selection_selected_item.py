"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_catalog_expectation import EnvironmentCatalogExpectation
from .model_base import Model


class EnvironmentCatalogSelections_EnvironmentCatalogSelectionSelectedItem(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    expected_catalog: EnvironmentCatalogExpectation | None = None
    id: StrictStr
    version: StrictStr

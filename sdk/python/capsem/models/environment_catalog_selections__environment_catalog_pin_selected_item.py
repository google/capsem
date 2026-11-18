"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model


class EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItem(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    catalog_reference: StrictStr
    id: StrictStr
    resolved_digest: StrictStr
    version: StrictStr

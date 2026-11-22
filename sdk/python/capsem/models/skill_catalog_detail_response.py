"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .catalog_entry_metadata import CatalogEntryMetadata
from .environment_catalog_pin import EnvironmentCatalogPin
from .model_base import Model


class SkillCatalogDetailResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    files: dict[str, StrictStr]
    metadata: CatalogEntryMetadata
    selection: EnvironmentCatalogPin

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .catalog_entry_metadata import CatalogEntryMetadata
from .environment_catalog_pin import EnvironmentCatalogPin
from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class EnvironmentSkillDetailResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    discovery_fresh: StrictBool
    files: dict[str, StrictStr]
    metadata: CatalogEntryMetadata
    revision: EnvironmentRevisionReference
    selection: EnvironmentCatalogPin

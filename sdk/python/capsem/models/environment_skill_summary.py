"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .catalog_entry_metadata import CatalogEntryMetadata
from .environment_catalog_pin import EnvironmentCatalogPin
from .environment_skill_availability import EnvironmentSkillAvailability
from .model_base import Model


class EnvironmentSkillSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    availability: EnvironmentSkillAvailability
    metadata: CatalogEntryMetadata | None = None
    selection: EnvironmentCatalogPin

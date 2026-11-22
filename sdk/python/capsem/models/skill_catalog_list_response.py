"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .catalog_entry_metadata import CatalogEntryMetadata
from .model_base import Model
from .skill_catalog_snapshot import SkillCatalogSnapshot


class SkillCatalogListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    catalog: SkillCatalogSnapshot
    next_cursor: StrictStr | None = None
    skills: list[CatalogEntryMetadata]

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .catalog_compatibility import CatalogCompatibility
from .catalog_entry_status import CatalogEntryStatus
from .model_base import Model


class CatalogEntryMetadata(Model):
    nonnullable_optional = frozenset(['description', 'tags'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    compatibility: CatalogCompatibility
    description: StrictStr | None = None
    id: StrictStr
    license: StrictStr
    name: StrictStr
    provenance: StrictStr
    publisher: StrictStr
    status: CatalogEntryStatus
    status_reason: StrictStr | None = None
    tags: list[StrictStr] | None = None
    version: StrictStr

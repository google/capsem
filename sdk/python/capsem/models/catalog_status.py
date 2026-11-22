"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictInt, StrictStr

from .catalog_availability import CatalogAvailability
from .catalog_kind import CatalogKind
from .catalog_refresh_error import CatalogRefreshError
from .catalog_source_origin import CatalogSourceOrigin
from .model_base import Model


class CatalogStatus(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    availability: CatalogAvailability
    digest: StrictStr | None
    error: CatalogRefreshError | None
    kind: CatalogKind
    last_successful_refresh: StrictInt | None
    origin: CatalogSourceOrigin | None
    reference: StrictStr | None

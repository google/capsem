"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .asset_manifest_status import AssetManifestStatus
from .model_base import Model
from .profile_catalog_source import ProfileCatalogSource
from .profile_readiness import ProfileReadiness


class ProfileCatalogStatus(Model):
    asset_manifest: AssetManifestStatus | None = None
    bytes_done: Annotated[StrictInt, Field(ge=0)] | None = None
    bytes_total: Annotated[StrictInt, Field(ge=0)] | None = None
    current_asset: StrictStr | None = None
    downloaded: Annotated[StrictInt, Field(ge=0)] | None = None
    profile_count: Annotated[StrictInt, Field(ge=0)]
    profiles: list[ProfileReadiness]
    ready_count: Annotated[StrictInt, Field(ge=0)]
    reconcile_error: StrictStr | None = None
    source: ProfileCatalogSource

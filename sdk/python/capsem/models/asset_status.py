"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .asset_file_status import AssetFileStatus
from .asset_manifest_status import AssetManifestStatus
from .model_base import Model


class AssetStatus(Model):
    asset_version: StrictStr | None = None
    assets: list[AssetFileStatus]
    bytes_done: Annotated[StrictInt, Field(ge=0)] | None = None
    bytes_total: Annotated[StrictInt, Field(ge=0)] | None = None
    current_arch: StrictStr
    current_asset: StrictStr | None = None
    downloaded: Annotated[StrictInt, Field(ge=0)] | None = None
    downloading: StrictBool
    errors: list[StrictStr]
    manifest: AssetManifestStatus
    ready: StrictBool
    reconcile_error: StrictStr | None = None
    started: StrictBool | None = None

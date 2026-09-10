"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .validation_status import ValidationStatus


class AssetManifestStatus(Model):
    assets_current: StrictStr | None = None
    binaries_current: StrictStr | None = None
    blake3: StrictStr | None = None
    format: Annotated[StrictInt, Field(ge=0)] | None = None
    origin: StrictStr
    origin_path: StrictStr | None = None
    origin_source: StrictStr | None = None
    packaged_at: StrictStr | None = None
    path: StrictStr
    refresh_policy: StrictStr | None = None
    refreshed_at: StrictStr | None = None
    validation_error: StrictStr | None = None
    validation_status: ValidationStatus

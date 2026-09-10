"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class StorageDiagnostics(Model):
    guest_overlay_device: StrictStr
    guest_overlay_mount: StrictStr
    host_available_bytes: Annotated[StrictInt, Field(ge=0)]
    host_free_bytes: Annotated[StrictInt, Field(ge=0)]
    host_total_bytes: Annotated[StrictInt, Field(ge=0)]
    rootfs_image_logical_bytes: Annotated[StrictInt, Field(ge=0)]
    rootfs_image_path: StrictStr
    rootfs_image_physical_bytes: Annotated[StrictInt, Field(ge=0)]

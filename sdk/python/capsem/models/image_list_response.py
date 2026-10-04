"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .catalog_info import CatalogInfo
from .image_info import ImageInfo
from .model_base import Model


class ImageListResponse(Model):
    catalog: CatalogInfo | None = None
    images: list[ImageInfo]

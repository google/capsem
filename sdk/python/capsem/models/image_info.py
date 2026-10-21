"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .image_cache_state import ImageCacheState
from .image_download_progress import ImageDownloadProgress
from .image_surface import ImageSurface
from .model_base import Model


class ImageInfo(Model):
    architectures: list[StrictStr]
    cached: ImageCacheState
    description: StrictStr
    image: StrictStr | None = None
    name: StrictStr
    progress: ImageDownloadProgress | None = None
    surface: ImageSurface | None = None

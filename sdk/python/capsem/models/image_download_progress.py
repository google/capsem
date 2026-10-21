"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .image_download_phase import ImageDownloadPhase
from .model_base import Model


class ImageDownloadProgress(Model):
    completed_bytes: Annotated[StrictInt, Field(ge=0)]
    phase: ImageDownloadPhase
    total_bytes: Annotated[StrictInt, Field(ge=0)] | None = None

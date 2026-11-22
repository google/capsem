"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_image import EnvironmentImage
from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class EnvironmentImageUpdateResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    available: EnvironmentImage | None = None
    current: EnvironmentImage | None = None
    environment: EnvironmentRevisionReference

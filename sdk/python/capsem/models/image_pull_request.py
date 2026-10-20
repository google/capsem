"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model
from .registry_access import RegistryAccess


class ImagePullRequest(Model):
    image: StrictStr
    registry: RegistryAccess | None = None

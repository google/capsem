"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictInt, StrictStr

from .environment_image import EnvironmentImage
from .model_base import Model


class EnvironmentListSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    description: StrictStr
    image: EnvironmentImage | None = None
    name: StrictStr
    timestamp_unix_ms: StrictInt

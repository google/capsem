"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .detection_level import DetectionLevel
from .model_base import Model


class EnvironmentDetectionRule(Model):
    nonnullable_optional = frozenset(['enabled'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    enabled: StrictBool | None = None
    level: DetectionLevel
    match: StrictStr
    name: StrictStr

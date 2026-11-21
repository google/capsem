"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .detection_level import DetectionLevel
from .environment_rule_source import EnvironmentRuleSource
from .model_base import Model


class EnvironmentEffectiveDetectionRule(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    condition: StrictStr
    editable_key: StrictStr | None
    enabled: StrictBool
    id: StrictStr
    level: DetectionLevel
    name: StrictStr
    source: EnvironmentRuleSource

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictInt, StrictStr

from .detection_level import DetectionLevel
from .environment_rule_source import EnvironmentRuleSource
from .model_base import Model
from .security_rule_action import SecurityRuleAction


class EnvironmentEffectiveAccessRule(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    action: SecurityRuleAction
    condition: StrictStr
    detection_level: DetectionLevel | None
    editable_key: StrictStr | None
    enabled: StrictBool
    id: StrictStr
    locked: StrictBool
    name: StrictStr
    priority: StrictInt
    source: EnvironmentRuleSource

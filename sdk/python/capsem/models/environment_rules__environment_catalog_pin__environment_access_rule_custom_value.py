"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictInt, StrictStr

from .environment_capability_decision import EnvironmentCapabilityDecision
from .model_base import Model


class EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRuleCustomValue(Model):
    nonnullable_optional = frozenset(['enabled'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    action: EnvironmentCapabilityDecision
    enabled: StrictBool | None = None
    match: StrictStr
    name: StrictStr
    priority: StrictInt | None = None

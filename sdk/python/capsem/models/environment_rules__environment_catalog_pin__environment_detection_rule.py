"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_catalog_selections__environment_catalog_pin import (
    EnvironmentCatalogSelections_EnvironmentCatalogPin,
)
from .environment_rule_parameter import EnvironmentRuleParameter
from .environment_rules__environment_catalog_pin__environment_detection_rule_custom_value import (
    EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValue,
)
from .model_base import Model


class EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule(Model):
    nonnullable_optional = frozenset(['catalog', 'custom', 'parameters'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    catalog: EnvironmentCatalogSelections_EnvironmentCatalogPin | None = None
    custom: dict[str, EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRuleCustomValue] | None = None
    parameters: dict[str, dict[str, EnvironmentRuleParameter]] | None = None

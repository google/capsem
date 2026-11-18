"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_catalog_selections__environment_catalog_selection import (
    EnvironmentCatalogSelections_EnvironmentCatalogSelection,
)
from .environment_rule_parameter import EnvironmentRuleParameter
from .environment_rules__environment_catalog_selection__environment_access_rule_custom_value import (
    EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue,
)
from .model_base import Model


class EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule(Model):
    nonnullable_optional = frozenset(['catalog', 'custom', 'parameters'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    catalog: EnvironmentCatalogSelections_EnvironmentCatalogSelection | None = None
    custom: dict[str, EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRuleCustomValue] | None = None
    parameters: dict[str, dict[str, EnvironmentRuleParameter]] | None = None

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .catalog_rule_parameter_boolean import CatalogRuleParameterBoolean
from .catalog_rule_parameter_choice import CatalogRuleParameterChoice
from .catalog_rule_parameter_integer import CatalogRuleParameterInteger
from .catalog_rule_parameter_text import CatalogRuleParameterText

CatalogRuleParameter: TypeAlias = CatalogRuleParameterBoolean | CatalogRuleParameterInteger | CatalogRuleParameterText | CatalogRuleParameterChoice

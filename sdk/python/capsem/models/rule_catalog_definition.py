"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .rule_catalog_definition_access import RuleCatalogDefinitionAccess
from .rule_catalog_definition_detection import RuleCatalogDefinitionDetection

RuleCatalogDefinition: TypeAlias = RuleCatalogDefinitionAccess | RuleCatalogDefinitionDetection

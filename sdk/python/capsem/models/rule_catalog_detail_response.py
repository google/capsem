"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .catalog_entry_metadata import CatalogEntryMetadata
from .catalog_rule_parameter import CatalogRuleParameter
from .environment_catalog_pin import EnvironmentCatalogPin
from .model_base import Model
from .rule_catalog_definition import RuleCatalogDefinition


class RuleCatalogDetailResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    definition: RuleCatalogDefinition
    metadata: CatalogEntryMetadata
    parameters: dict[str, CatalogRuleParameter]
    selection: EnvironmentCatalogPin

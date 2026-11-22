"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .catalog_entry_metadata import CatalogEntryMetadata
from .catalog_rule_kind import CatalogRuleKind
from .model_base import Model


class RuleCatalogSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    kind: CatalogRuleKind
    metadata: CatalogEntryMetadata

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .catalog_snapshot import CatalogSnapshot
from .model_base import Model
from .rule_catalog_summary import RuleCatalogSummary


class RuleCatalogListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    catalog: CatalogSnapshot
    next_cursor: StrictStr | None = None
    rules: list[RuleCatalogSummary]

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .catalog_snapshot import CatalogSnapshot
from .mcp_catalog_summary import McpCatalogSummary
from .model_base import Model


class McpCatalogListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    catalog: CatalogSnapshot
    mcps: list[McpCatalogSummary]
    next_cursor: StrictStr | None = None

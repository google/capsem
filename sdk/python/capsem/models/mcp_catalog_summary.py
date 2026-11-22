"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .catalog_entry_metadata import CatalogEntryMetadata
from .mcp_catalog_kind import McpCatalogKind
from .model_base import Model


class McpCatalogSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    kind: McpCatalogKind
    metadata: CatalogEntryMetadata

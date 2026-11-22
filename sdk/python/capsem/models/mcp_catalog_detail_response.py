"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .catalog_entry_metadata import CatalogEntryMetadata
from .environment_catalog_pin import EnvironmentCatalogPin
from .mcp_catalog_definition import McpCatalogDefinition
from .mcp_published_tool import McpPublishedTool
from .model_base import Model


class McpCatalogDetailResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    definition: McpCatalogDefinition
    metadata: CatalogEntryMetadata
    published_tools: list[McpPublishedTool] | None = None
    selection: EnvironmentCatalogPin

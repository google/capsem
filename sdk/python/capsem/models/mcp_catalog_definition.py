"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .mcp_catalog_definition_remote import McpCatalogDefinitionRemote
from .mcp_catalog_definition_sandbox import McpCatalogDefinitionSandbox

McpCatalogDefinition: TypeAlias = McpCatalogDefinitionRemote | McpCatalogDefinitionSandbox

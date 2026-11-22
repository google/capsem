"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictStr

from .mcp_catalog_requirement import McpCatalogRequirement
from .model_base import Model


class McpCatalogDefinitionRemote(Model):
    nonnullable_optional = frozenset(['headers'])
    headers: dict[str, McpCatalogRequirement] | None = None
    kind: Literal["remote"]
    secret_binding: McpCatalogRequirement | None = None
    url: StrictStr

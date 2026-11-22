"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_catalog_selections__environment_catalog_selection import (
    EnvironmentCatalogSelections_EnvironmentCatalogSelection,
)
from .environment_mcp_definition import EnvironmentMcpDefinition
from .environment_mcp_permissions import EnvironmentMcpPermissions
from .environment_mcps__environment_catalog_selection__environment_mcp_catalog_input_value import (
    EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue,
)
from .model_base import Model


class EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput(Model):
    nonnullable_optional = frozenset(['catalog', 'permissions', 'servers'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    catalog: EnvironmentCatalogSelections_EnvironmentCatalogSelection | None = None
    catalog_configuration: dict[str, EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue] | None = None
    permissions: dict[str, EnvironmentMcpPermissions] | None = None
    servers: dict[str, EnvironmentMcpDefinition] | None = None

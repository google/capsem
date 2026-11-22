"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_catalog_selections__environment_catalog_pin import (
    EnvironmentCatalogSelections_EnvironmentCatalogPin,
)
from .environment_mcp_definition import EnvironmentMcpDefinition
from .environment_mcp_permissions import EnvironmentMcpPermissions
from .environment_mcps__environment_catalog_pin__environment_mcp_definition_value_alternative0 import (
    EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0,
)
from .environment_mcps__environment_catalog_pin__environment_mcp_definition_value_alternative1 import (
    EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1,
)
from .model_base import Model


class EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition(Model):
    nonnullable_optional = frozenset(['catalog', 'permissions', 'servers'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    catalog: EnvironmentCatalogSelections_EnvironmentCatalogPin | None = None
    catalog_configuration: dict[str, EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0 | EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1] | None = None
    permissions: dict[str, EnvironmentMcpPermissions] | None = None
    servers: dict[str, EnvironmentMcpDefinition] | None = None

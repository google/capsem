"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_catalog_selections__environment_catalog_pin import (
    EnvironmentCatalogSelections_EnvironmentCatalogPin,
)
from .environment_mcps__environment_catalog_pin__environment_mcp_definition import (
    EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition,
)
from .environment_packages import EnvironmentPackages
from .environment_plugins import EnvironmentPlugins
from .environment_rules__environment_catalog_pin__environment_access_rule import (
    EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule,
)
from .environment_rules__environment_catalog_pin__environment_detection_rule import (
    EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule,
)
from .environment_secret_binding import EnvironmentSecretBinding
from .environment_specification__environment_image__environment_repository__environment_catalog_pin__environment_mcp_definition_image import (
    EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage,
)
from .environment_specification__environment_image__environment_repository__environment_catalog_pin__environment_mcp_definition_repositories_item import (
    EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem,
)
from .environment_startup import EnvironmentStartup
from .model_base import Model


class EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition(Model):
    nonnullable_optional = frozenset(['access', 'description', 'detection', 'image', 'mcps', 'packages', 'plugins', 'repositories', 'secrets', 'skills', 'variables'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    access: EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule | None = None
    description: StrictStr | None = None
    detection: EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule | None = None
    image: EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionImage | None = None
    mcps: EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition | None = None
    name: StrictStr
    packages: EnvironmentPackages | None = None
    plugins: EnvironmentPlugins | None = None
    repositories: list[EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinitionRepositoriesItem] | None = None
    secrets: dict[str, EnvironmentSecretBinding] | None = None
    skills: EnvironmentCatalogSelections_EnvironmentCatalogPin | None = None
    startup: EnvironmentStartup | None = None
    variables: dict[str, StrictStr] | None = None

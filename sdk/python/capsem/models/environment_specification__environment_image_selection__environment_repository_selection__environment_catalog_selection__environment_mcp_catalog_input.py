"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_catalog_selections__environment_catalog_selection import (
    EnvironmentCatalogSelections_EnvironmentCatalogSelection,
)
from .environment_mcps__environment_catalog_selection__environment_mcp_catalog_input import (
    EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput,
)
from .environment_packages import EnvironmentPackages
from .environment_plugins import EnvironmentPlugins
from .environment_rules__environment_catalog_selection__environment_access_rule import (
    EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule,
)
from .environment_rules__environment_catalog_selection__environment_detection_rule import (
    EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule,
)
from .environment_secret_binding import EnvironmentSecretBinding
from .environment_specification__environment_image_selection__environment_repository_selection__environment_catalog_selection__environment_mcp_catalog_input_image import (
    EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputImage,
)
from .environment_specification__environment_image_selection__environment_repository_selection__environment_catalog_selection__environment_mcp_catalog_input_repositories_item import (
    EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputRepositoriesItem,
)
from .environment_startup import EnvironmentStartup
from .model_base import Model


class EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput(Model):
    nonnullable_optional = frozenset(['access', 'description', 'detection', 'image', 'mcps', 'packages', 'plugins', 'repositories', 'secrets', 'skills', 'variables'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    access: EnvironmentRules_EnvironmentCatalogSelection_EnvironmentAccessRule | None = None
    description: StrictStr | None = None
    detection: EnvironmentRules_EnvironmentCatalogSelection_EnvironmentDetectionRule | None = None
    image: EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputImage | None = None
    mcps: EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput | None = None
    name: StrictStr
    packages: EnvironmentPackages | None = None
    plugins: EnvironmentPlugins | None = None
    repositories: list[EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputRepositoriesItem] | None = None
    secrets: dict[str, EnvironmentSecretBinding] | None = None
    skills: EnvironmentCatalogSelections_EnvironmentCatalogSelection | None = None
    startup: EnvironmentStartup | None = None
    variables: dict[str, StrictStr] | None = None

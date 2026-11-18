"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_catalog_selections__environment_catalog_pin import (
    EnvironmentCatalogSelections_EnvironmentCatalogPin,
)
from .environment_mcps__environment_catalog_pin import (
    EnvironmentMcps_EnvironmentCatalogPin,
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
from .environment_specification__environment_image__environment_repository__environment_catalog_pin_image import (
    EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage,
)
from .environment_specification__environment_image__environment_repository__environment_catalog_pin_repositories_item import (
    EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem,
)
from .environment_startup import EnvironmentStartup
from .model_base import Model


class EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin(Model):
    nonnullable_optional = frozenset(['access', 'description', 'detection', 'image', 'mcps', 'packages', 'plugins', 'repositories', 'secrets', 'skills', 'variables'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    access: EnvironmentRules_EnvironmentCatalogPin_EnvironmentAccessRule | None = None
    description: StrictStr | None = None
    detection: EnvironmentRules_EnvironmentCatalogPin_EnvironmentDetectionRule | None = None
    image: EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinImage | None = None
    mcps: EnvironmentMcps_EnvironmentCatalogPin | None = None
    name: StrictStr
    packages: EnvironmentPackages | None = None
    plugins: EnvironmentPlugins | None = None
    repositories: list[EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem] | None = None
    secrets: dict[str, EnvironmentSecretBinding] | None = None
    skills: EnvironmentCatalogSelections_EnvironmentCatalogPin | None = None
    startup: EnvironmentStartup | None = None
    variables: dict[str, StrictStr] | None = None

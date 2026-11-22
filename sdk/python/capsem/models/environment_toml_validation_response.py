"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_revision_reference import EnvironmentRevisionReference
from .environment_specification__environment_image_selection__environment_repository_selection__environment_catalog_selection__environment_mcp_catalog_input import (
    EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput,
)
from .model_base import Model


class EnvironmentTomlValidationResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    base: EnvironmentRevisionReference
    specification: EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput

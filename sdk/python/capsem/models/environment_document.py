"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_identity import EnvironmentIdentity
from .environment_specification__environment_image__environment_repository__environment_catalog_pin__environment_mcp_definition import (
    EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition,
)
from .model_base import Model


class EnvironmentDocument(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    identity: EnvironmentIdentity
    specification: EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition

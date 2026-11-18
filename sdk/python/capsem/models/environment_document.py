"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_identity import EnvironmentIdentity
from .environment_specification__environment_image__environment_repository__environment_catalog_pin import (
    EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin,
)
from .model_base import Model


class EnvironmentDocument(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    identity: EnvironmentIdentity
    specification: EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin

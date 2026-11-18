"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_sandbox_target import EnvironmentSandboxTarget
from .environment_specification__environment_image_selection__environment_repository_selection__environment_catalog_selection import (
    EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection,
)
from .model_base import Model, UuidString


class EnvironmentCreateRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    reason: StrictStr
    request_id: UuidString
    requesting_sandbox: EnvironmentSandboxTarget | None = None
    specification: EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelection

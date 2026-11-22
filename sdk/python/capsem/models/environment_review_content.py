"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_specification__environment_image__environment_repository__environment_catalog_pin__environment_mcp_definition import (
    EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition,
)
from .model_base import Model
from .request_review_availability import RequestReviewAvailability


class EnvironmentReviewContent(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    availability: RequestReviewAvailability
    purged_variables: list[StrictStr]
    specification: EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition | None = None

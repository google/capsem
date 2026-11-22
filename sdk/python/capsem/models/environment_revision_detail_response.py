"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_revision_summary import EnvironmentRevisionSummary
from .environment_specification__environment_image__environment_repository__environment_catalog_pin__environment_mcp_definition import (
    EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition,
)
from .model_base import Model
from .request_review_availability import RequestReviewAvailability


class EnvironmentRevisionDetailResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    availability: RequestReviewAvailability
    canonical_toml: StrictStr | None = None
    partial_specification: EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPin_EnvironmentMcpDefinition | None = None
    purged_variables: list[StrictStr]
    summary: EnvironmentRevisionSummary

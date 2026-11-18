"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model


class EnvironmentSpecification_EnvironmentImage_EnvironmentRepository_EnvironmentCatalogPinRepositoriesItem(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    destination: StrictStr
    ref: StrictStr
    resolved_commit: StrictStr
    url: StrictStr

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_image_update_policy import EnvironmentImageUpdatePolicy
from .model_base import Model


class EnvironmentSpecification_EnvironmentImageSelection_EnvironmentRepositorySelection_EnvironmentCatalogSelectionImage(Model):
    nonnullable_optional = frozenset(['update_policy'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    reference: StrictStr
    update_policy: EnvironmentImageUpdatePolicy | None = None

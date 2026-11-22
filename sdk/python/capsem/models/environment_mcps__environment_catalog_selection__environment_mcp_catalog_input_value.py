"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_secret_binding import EnvironmentSecretBinding
from .model_base import Model


class EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue(Model):
    nonnullable_optional = frozenset(['secrets', 'values'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    binding_id: StrictStr | None = None
    secrets: dict[str, EnvironmentSecretBinding] | None = None
    values: dict[str, StrictStr] | None = None

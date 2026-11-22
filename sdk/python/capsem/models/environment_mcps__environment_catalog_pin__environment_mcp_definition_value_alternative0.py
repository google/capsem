"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictStr

from .model_base import Model


class EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0(Model):
    nonnullable_optional = frozenset(['headers'])
    binding_id: StrictStr | None = None
    headers: dict[str, StrictStr] | None = None
    kind: Literal["remote"]
    url: StrictStr

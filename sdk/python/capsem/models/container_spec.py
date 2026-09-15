"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model
from .registry_access import RegistryAccess


class ContainerSpec(Model):
    nonnullable_optional = frozenset(['args', 'env'])
    args: list[StrictStr] | None = None
    env: dict[str, StrictStr] | None = None
    image: StrictStr
    registry: RegistryAccess | None = None

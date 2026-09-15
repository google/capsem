"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr

from .model_base import Model
from .registry_access import RegistryAccess


class ContainerSpec(Model):
    nonnullable_optional = frozenset(['args', 'attach', 'env'])
    args: list[StrictStr] | None = None
    attach: StrictBool | None = None
    env: dict[str, StrictStr] | None = None
    image: StrictStr
    registry: RegistryAccess | None = None

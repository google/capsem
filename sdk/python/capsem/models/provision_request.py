"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictBool, StrictInt, StrictStr

from .container_spec import ContainerSpec
from .managed_session_create import ManagedSessionCreate
from .model_base import Model


class ProvisionRequest(Model):
    private_input = True
    nonnullable_optional = frozenset(['networks', 'persistent'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid", hide_input_in_errors=True)
    container: ContainerSpec | None = None
    cpus: Annotated[StrictInt, Field(ge=0)] | None = None
    env: dict[str, StrictStr] | None = None
    from_: StrictStr | None = Field(default=None, alias='from')
    managed: ManagedSessionCreate | None = None
    name: StrictStr | None = None
    networks: list[StrictStr] | None = None
    persistent: StrictBool | None = None
    ram_mb: Annotated[StrictInt, Field(ge=0)] | None = None

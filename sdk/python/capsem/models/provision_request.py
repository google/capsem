"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictBool, StrictInt, StrictStr

from .container_spec import ContainerSpec
from .environment_revision_reference import EnvironmentRevisionReference
from .managed_session_create import ManagedSessionCreate
from .model_base import Model
from .vm_lifecycle_selection import VmLifecycleSelection


class ProvisionRequest(Model):
    private_input = True
    nonnullable_optional = frozenset(['attach', 'networks', 'persistent'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid", hide_input_in_errors=True)
    attach: StrictBool | None = None
    container: ContainerSpec | None = None
    cpus: Annotated[StrictInt, Field(ge=0)] | None = None
    env: dict[str, StrictStr] | None = None
    environment: EnvironmentRevisionReference | None = None
    expected_source: VmLifecycleSelection | None = None
    from_: StrictStr | None = Field(default=None, alias='from')
    labels: dict[str, StrictStr] | None = None
    managed: ManagedSessionCreate | None = None
    name: StrictStr | None = None
    networks: list[StrictStr] | None = None
    persistent: StrictBool | None = None
    ram_mb: Annotated[StrictInt, Field(ge=0)] | None = None

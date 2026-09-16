"""Typed facade options that are distinct from gateway wire models."""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict, Field, StrictBool, StrictStr

from .models import RegistryAccess


class ContainerOptions(BaseModel):
    """Container workload settings; pass its environment to ``Hypervisor.create``."""

    model_config = ConfigDict(strict=True, extra="forbid")

    image: StrictStr
    args: list[StrictStr] = Field(default_factory=list)
    registry: RegistryAccess | None = None
    attach: StrictBool = False

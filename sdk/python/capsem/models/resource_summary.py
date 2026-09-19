"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .model_base import Model


class ResourceSummary(Model):
    running_count: Annotated[StrictInt, Field(ge=0)]
    stopped_count: Annotated[StrictInt, Field(ge=0)]
    suspended_count: Annotated[StrictInt, Field(ge=0)]
    total_cpus: Annotated[StrictInt, Field(ge=0)]
    total_ram_mb: Annotated[StrictInt, Field(ge=0)]

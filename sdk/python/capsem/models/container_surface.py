"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .container_surface_kind import ContainerSurfaceKind
from .model_base import Model


class ContainerSurface(Model):
    exposure_id: StrictStr | None = None
    kind: ContainerSurfaceKind
    port: Annotated[StrictInt, Field(ge=0)]

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .exposure_access import ExposureAccess
from .exposure_target import ExposureTarget
from .model_base import Model


class ExposureRequest(Model):
    nonnullable_optional = frozenset(['access', 'host_port', 'target'])
    access: ExposureAccess | None = None
    guest_port: Annotated[StrictInt, Field(ge=0)]
    host_port: Annotated[StrictInt, Field(ge=0)] | None = None
    target: ExposureTarget | None = None

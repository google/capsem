"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .exposure_target import ExposureTarget
from .model_base import Model


class ExposureInfo(Model):
    guest_port: Annotated[StrictInt, Field(ge=0)]
    host_port: Annotated[StrictInt, Field(ge=0)]
    id: StrictStr
    target: ExposureTarget

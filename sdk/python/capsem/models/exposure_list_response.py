"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .exposure_info import ExposureInfo
from .model_base import Model


class ExposureListResponse(Model):
    exposures: list[ExposureInfo]
    owner_generation: StrictStr

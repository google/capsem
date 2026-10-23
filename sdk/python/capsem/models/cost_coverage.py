"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .model_base import Model


class CostCoverage(Model):
    priced_calls: Annotated[StrictInt, Field(ge=0)]
    unknown_calls: Annotated[StrictInt, Field(ge=0)]
    unpriced_calls: Annotated[StrictInt, Field(ge=0)]

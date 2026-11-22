"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt

from .model_base import Model


class CatalogCompatibility(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    min_runtime_contract: Annotated[StrictInt, Field(ge=0)]

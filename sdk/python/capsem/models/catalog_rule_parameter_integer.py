"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictInt

from .model_base import Model


class CatalogRuleParameterInteger(Model):
    default: StrictInt
    max: StrictInt
    min: StrictInt
    type: Literal["integer"]

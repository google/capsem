"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictBool

from .model_base import Model


class CatalogRuleParameterBoolean(Model):
    default: StrictBool
    type: Literal["boolean"]

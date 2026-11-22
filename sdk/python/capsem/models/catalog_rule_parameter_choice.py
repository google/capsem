"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictStr

from .model_base import Model


class CatalogRuleParameterChoice(Model):
    choices: list[StrictStr]
    default: StrictStr
    type: Literal["choice"]

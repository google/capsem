"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated, Literal

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model


class CatalogRuleParameterText(Model):
    nonnullable_optional = frozenset(['allow_empty'])
    allow_empty: StrictBool | None = None
    default: StrictStr
    max_bytes: Annotated[StrictInt, Field(ge=0)]
    type: Literal["text"]

"""Shared validation for JSON values and optional nonnullable fields."""

from __future__ import annotations

from math import isfinite
from typing import Annotated, ClassVar, TypeAlias

from pydantic import AfterValidator, BaseModel, ConfigDict, model_validator
from pydantic import JsonValue as PydanticJsonValue


def _finite_json(value: PydanticJsonValue) -> PydanticJsonValue:
    pending = [value]
    while pending:
        item = pending.pop()
        if isinstance(item, float) and not isfinite(item):
            raise ValueError("JSON numbers must be finite")
        if isinstance(item, dict):
            pending.extend(item.values())
        elif isinstance(item, list):
            pending.extend(item)
    return value


JsonValue: TypeAlias = Annotated[PydanticJsonValue, AfterValidator(_finite_json)]


class Model(BaseModel):
    model_config = ConfigDict(strict=True, populate_by_name=True)
    nonnullable_optional: ClassVar[frozenset[str]] = frozenset()

    @model_validator(mode="before")
    @classmethod
    def reject_explicit_null(cls, value: object) -> object:
        if isinstance(value, dict):
            for key in cls.nonnullable_optional:
                if key in value and value[key] is None:
                    raise ValueError(f"{key} may be omitted but cannot be null")
        return value

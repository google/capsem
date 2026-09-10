"""Shared validation for optional fields that do not permit JSON null."""

from __future__ import annotations

from typing import ClassVar

from pydantic import BaseModel, ConfigDict, model_validator


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

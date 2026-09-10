"""The OpenAPI schema vocabulary Capsem exports, validated before rendering.

This is intentionally not a general OpenAPI implementation. New constructs
must gain generator support and tests instead of falling back to untyped data.
"""

from __future__ import annotations

import json
from graphlib import CycleError, TopologicalSorter
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

Primitive = Literal["object", "array", "string", "integer", "number", "boolean", "null"]


class Schema(BaseModel):
    """Keep optional presence separate from a property's nullable type."""

    model_config = ConfigDict(extra="forbid", strict=True, populate_by_name=True)

    ref: str | None = Field(default=None, alias="$ref", pattern=r"^#/components/schemas/\w+$")
    type: Primitive | list[Primitive] | None = None
    description: str | None = None
    format: Literal["int32", "int64", "double", "binary"] | None = None
    minimum: int | float | None = None
    enum: list[str] | None = None
    properties: dict[str, Schema] = Field(default_factory=dict)
    required: list[str] = Field(default_factory=list)
    items: Schema | None = None
    one_of: list[Schema] | None = Field(default=None, alias="oneOf")
    additional_properties: Schema | Literal[False] | None = Field(
        default=None, alias="additionalProperties",
    )
    property_names: Schema | None = Field(default=None, alias="propertyNames")

    @model_validator(mode="after")
    def supported_shape(self) -> Schema:
        fields = self.model_fields_set - {"description"}
        if self.ref is not None:
            if fields != {"ref"}:
                raise ValueError("reference may only carry a description")
            return self
        if self.one_of is not None:
            if fields != {"one_of"} or len(self.one_of) < 2:
                raise ValueError("oneOf needs at least two alternatives and no sibling constraints")
            return self
        kinds = self.type if isinstance(self.type, list) else [self.type]
        if not kinds:
            raise ValueError("type array must not be empty")
        if len(kinds) > 1 and (len(kinds) != 2 or "null" not in kinds or kinds[0] == kinds[1]):
            raise ValueError("only nullable type arrays are supported")
        kind = next((value for value in kinds if value != "null"), "null")
        if kind is None:
            raise ValueError("schema requires a type, reference or union")
        allowed = {"type"}
        if kind == "object":
            allowed |= {"properties", "required", "additional_properties", "property_names"}
            if set(self.required) - self.properties.keys():
                raise ValueError("required field has no property schema")
            if self.properties and isinstance(self.additional_properties, Schema):
                raise ValueError("mixed named properties and map entries are unsupported")
            if (self.property_names is not None
                    and self.property_names.model_dump(exclude_unset=True) != {"type": "string"}):
                raise ValueError("map keys must be unconstrained strings")
        elif kind == "array":
            allowed.add("items")
            if self.items is None:
                raise ValueError("array requires an item schema")
        elif kind == "string":
            allowed |= {"enum", "format"}
            if self.enum is not None and (not self.enum or len(set(self.enum)) != len(self.enum)):
                raise ValueError("enum must contain distinct values")
            if self.format not in (None, "binary"):
                raise ValueError("unsupported string format")
        elif kind in ("integer", "number"):
            allowed |= {"minimum", "format"}
            formats = (None, "int32", "int64") if kind == "integer" else (None, "double")
            if self.format not in formats:
                raise ValueError("numeric format does not match type")
        if fields - allowed:
            raise ValueError(f"unsupported constraints for {kind}: {sorted(fields - allowed)}")
        return self

    def references(self) -> set[str]:
        """Collect dependencies through object properties, maps, arrays and unions."""
        if self.ref is not None:
            return {self.ref.rsplit("/", 1)[1]}
        children = [*self.properties.values(), *(self.one_of or [])]
        children.extend(child for child in (self.items, self.additional_properties) if isinstance(child, Schema))
        return {name for child in children for name in child.references()}


def schema_order(schemas: dict[str, Schema]) -> list[str]:
    """Dependencies precede their consumers; unresolved or cyclic refs fail."""
    graph = {
        name: sorted(schema.references() - ({name} if schema.type == "object" else set()))
        for name, schema in sorted(schemas.items())
    }
    missing = {ref for refs in graph.values() for ref in refs} - schemas.keys()
    if missing:
        raise ValueError(f"missing schema references: {sorted(missing)}")
    try:
        return list(TopologicalSorter(graph).static_order())
    except CycleError as error:
        raise ValueError(f"schema reference cycle: {error.args[1]}") from error


def read_schemas(path: Path) -> dict[str, Schema]:
    document = json.loads(path.read_text())
    schemas = {
        name: Schema.model_validate(value)
        for name, value in document["components"]["schemas"].items()
    }
    if not schemas:
        raise ValueError("OpenAPI document contains no schemas")
    schema_order(schemas)
    return schemas

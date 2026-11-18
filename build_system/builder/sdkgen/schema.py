"""The OpenAPI schema vocabulary Capsem exports, validated before rendering.

This is intentionally not a general OpenAPI implementation. New constructs
must gain generator support and tests instead of falling back to untyped data.
"""

from __future__ import annotations

import json
import re
from graphlib import CycleError, TopologicalSorter
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, JsonValue, model_validator

Primitive = Literal["object", "array", "string", "integer", "number", "boolean", "null"]


class Schema(BaseModel):
    """Keep optional presence separate from a property's nullable type."""

    model_config = ConfigDict(extra="forbid", strict=True, populate_by_name=True)

    ref: str | None = Field(default=None, alias="$ref", pattern=r"^#/components/schemas/\w+$")
    type: Primitive | list[Primitive] | None = None
    description: str | None = None
    # Informational only: never synthesize an omitted optional property.
    default_value: JsonValue = Field(default=None, alias="default")
    write_only: bool = Field(default=False, alias="writeOnly")
    format: Literal["int32", "int64", "double", "binary", "ipv4", "uuid", "date-time"] | None = None
    pattern: str | None = None
    minimum: int | float | None = None
    maximum: int | float | None = None
    max_length: int | None = Field(default=None, alias="maxLength", ge=0)
    max_items: int | None = Field(default=None, alias="maxItems", ge=0)
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
        nullable = self.type == "null" or (isinstance(self.type, list) and "null" in self.type) or any(
            branch.type == "null" for branch in self.one_of or []
        )
        if "default_value" in self.model_fields_set and self.default_value is None and not nullable:
            raise ValueError("null default requires an explicitly nullable schema")
        fields = self.model_fields_set - {"description", "default_value"}
        # An explicitly empty schema is arbitrary JSON, including JSON null.
        # Unknown constraints still fail validation; this is not an error fallback.
        if not fields:
            return self
        if self.ref is not None:
            if fields != {"ref"}:
                raise ValueError("reference may only carry a description")
            return self
        if self.one_of is not None:
            annotated_reference = (len(self.one_of) == 1 and self.one_of[0].ref is not None
                                   and "default_value" in self.model_fields_set)
            if fields != {"one_of"} or (len(self.one_of) < 2 and not annotated_reference):
                raise ValueError("oneOf needs alternatives or an annotated reference, without sibling constraints")
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
            allowed |= {"items", "max_items"}
            if self.items is None:
                raise ValueError("array requires an item schema")
        elif kind == "string":
            allowed |= {"enum", "format", "write_only", "pattern", "max_length"}
            if self.enum is not None and (not self.enum or len(set(self.enum)) != len(self.enum)):
                raise ValueError("enum must contain distinct values")
            if self.format not in (None, "binary", "ipv4", "uuid", "date-time"):
                raise ValueError("unsupported string format")
            if self.pattern is not None:
                if self.format is not None or self.pattern_width is None or self.pattern_width > 4096:
                    raise ValueError("only bounded, anchored character-class string patterns are supported")
                try:
                    re.compile(self.pattern)
                except re.error as error:
                    raise ValueError("invalid string character-class pattern") from error
        elif kind in ("integer", "number"):
            allowed |= {"minimum", "maximum", "format"}
            if self.minimum is not None and self.maximum is not None and self.minimum > self.maximum:
                raise ValueError("numeric minimum exceeds maximum")
            formats = (None, "int32", "int64") if kind == "integer" else (None, "double")
            if self.format not in formats:
                raise ValueError("numeric format does not match type")
        if fields - allowed:
            raise ValueError(f"unsupported constraints for {kind}: {sorted(fields - allowed)}")
        return self

    @property
    def pattern_width(self) -> int | None:
        match = re.fullmatch(r"\^\[[0-9A-Za-z-]+\]\{([1-9][0-9]{0,3})\}\$", self.pattern or "")
        return int(match[1]) if match else None

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
    name_tagged_variants(schemas)
    name_inline_objects(schemas)
    schema_order(schemas)
    return schemas


def name_tagged_variants(schemas: dict[str, Schema]) -> None:
    """Give inline tagged Rust variants stable model names for every SDK.

    The discriminator supplies identity; source order cannot rename a model.
    Other inline objects remain unsupported rather than becoming untyped data.
    """
    for name, schema in list(schemas.items()):
        if schema.one_of is None:
            continue
        members = []
        seen = set()
        for member in schema.one_of:
            if member.type != "object" or not member.properties:
                members.append(member)
                continue
            tags = [field.enum[0] for key, field in member.properties.items()
                    if key in {"type", "kind"} and key in member.required
                    and field.type == "string" and field.enum and len(field.enum) == 1]
            if len(tags) != 1 or not re.fullmatch(r"[a-z][a-z0-9_]*", tags[0]):
                raise ValueError(f"inline union {name} needs one required literal discriminator")
            variant = name + "".join(word.capitalize() for word in tags[0].split("_"))
            if variant in seen or (variant in schemas and schemas[variant] != member):
                raise ValueError(f"tagged union model collision: {variant}")
            seen.add(variant)
            schemas[variant] = member
            members.append(Schema(ref=f"#/components/schemas/{variant}"))
        schemas[name] = schema.model_copy(update={"one_of": members})


def name_inline_objects(schemas: dict[str, Schema]) -> None:
    """Lift Rust generic inline object members without changing their shape.

    Names derive from the containing component/property path, not source order
    or a content hash. Ambiguous names fail instead of overwriting a model.
    """
    def property_suffix(key: str) -> str:
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            raise ValueError("unsafe inline model property path")
        suffix = "".join(part[:1].upper() + part[1:] for part in key.split("_"))
        if not suffix:
            raise ValueError("empty inline model property path")
        return suffix

    def visit(schema: Schema, path: str, *, root: bool = False) -> Schema:
        named_object = schema.type == "object" and not isinstance(schema.additional_properties, Schema)
        named_enum = schema.enum is not None and len(schema.enum) > 1
        if not root and (named_object or named_enum):
            if path in schemas:
                raise ValueError(f"inline model name collision: {path}")
            schemas[path] = schema
            visit(schema, path, root=True)
            return Schema.model_validate({"$ref": f"#/components/schemas/{path}"})
        for key, child in list(schema.properties.items()):
            # Preserve wire property spelling; the suffix only names a new
            # model, and is needed solely when there is a nested object.
            nested = (child.type == "object" or child.items is not None or child.one_of is not None
                      or (child.enum is not None and len(child.enum) > 1))
            suffix = property_suffix(key) if nested else ""
            schema.properties[key] = visit(child, path + suffix)
        if schema.items is not None:
            schema.items = visit(schema.items, path + "Item")
        if isinstance(schema.additional_properties, Schema):
            schema.additional_properties = visit(schema.additional_properties, path + "Value")
        if schema.one_of is not None:
            schema.one_of = [visit(child, path + f"Alternative{index}")
                             for index, child in enumerate(schema.one_of)]
        return schema

    for name, schema in list(schemas.items()):
        visit(schema, name, root=True)

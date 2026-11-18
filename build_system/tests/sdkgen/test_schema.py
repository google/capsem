"""The generator must preserve the wire contract or reject it explicitly."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from capsem_builder.sdkgen.schema import Schema, read_schemas, schema_order
from pydantic import ValidationError

ROOT = Path(__file__).resolve().parents[3]


def test_actual_export_preserves_every_schema() -> None:
    path = ROOT / "sdk/specification/openapi.json"
    original = json.loads(path.read_text())["components"]["schemas"]
    schemas = read_schemas(path)
    assert set(original) <= set(schema_order(schemas))
    lifted = set(schemas) - original.keys()

    def expand(value: object) -> object:
        if isinstance(value, list):
            return [expand(child) for child in value]
        if isinstance(value, dict):
            ref = value.get("$ref", "").rsplit("/", 1)[-1]
            if ref in lifted:
                return expand(schemas[ref].model_dump(by_alias=True, exclude_unset=True))
            return {key: expand(child) for key, child in value.items()}
        return value

    for name, value in original.items():
        rendered = schemas[name].model_dump(by_alias=True, exclude_unset=True)
        assert expand(rendered) == value



@pytest.mark.parametrize("value", [
    {"type": "string", "pattern": "[a-z]+"},
    {"type": "string", "pattern": "^[a-z]{4097}$"},
    {"type": "string", "pattern": "^[z-a]{64}$"},
    {"type": "string", "format": "binary", "pattern": "^[a-z]{64}$"},
    {"type": "string", "format": "int32"},
    {"type": "string", "minimum": 0},
    {"type": "string", "maximum": 1},
    {"type": "integer", "minimum": 2, "maximum": 1},
    {"type": "integer", "format": "double"},
    {"type": ["integer", "string"]},
    {"type": []},
    {"type": "array"},
    {"type": "object", "required": ["missing"]},
    {"type": "object", "additionalProperties": True},
    {"type": "object", "propertyNames": {"type": "integer"}},
    {"type": "object", "properties": {"x": {"type": "string"}},
     "additionalProperties": {"type": "string"}},
    {"$ref": "https://example.org/schema"},
    {"$ref": "#/components/schemas/Name", "type": "string"},
    {"type": "string", "enum": []},
    {"type": "string", "enum": ["x", "x"]},
    {"oneOf": [{"type": "string"}]},
    {"oneOf": [{"type": "string"}, {"type": "null"}], "type": "string"},
    {"type": "string", "items": {"type": "string"}},
])
def test_unsupported_or_ambiguous_schema_fails(value: object) -> None:
    with pytest.raises(ValidationError):
        Schema.model_validate(value)


def test_missing_reference_fails_before_rendering() -> None:
    schemas = {"Example": Schema.model_validate({"$ref": "#/components/schemas/Missing"})}
    with pytest.raises(ValueError, match="Missing"):
        schema_order(schemas)


def test_dependencies_inside_nullable_union_and_map_are_ordered() -> None:
    schemas = {
        "Container": Schema.model_validate({"type": "object", "properties": {
            "values": {"type": "object", "additionalProperties": {
                "type": "array", "items": {"oneOf": [
                    {"type": "null"}, {"$ref": "#/components/schemas/Value"},
                ]},
            }},
        }}),
        "Value": Schema.model_validate({"type": "string", "enum": ["allowed", "denied"]}),
    }
    assert schema_order(schemas) == ["Value", "Container"]


def test_optional_is_distinct_from_nullable() -> None:
    schema = Schema.model_validate({"type": "object", "properties": {
        "nullable": {"type": ["integer", "null"], "format": "int64", "minimum": 0},
        "optional": {"type": "string"},
    }, "required": ["nullable"]})
    assert schema.required == ["nullable"]
    assert schema.properties["nullable"].type == ["integer", "null"]
    assert schema.properties["nullable"].minimum == 0
    assert schema.properties["optional"].type == "string"


@pytest.mark.parametrize("value", [
    {"type": ["string", "null"], "format": "date-time"},
    {"type": "integer", "minimum": 0, "maximum": 9007199254740991},
])
def test_connection_timestamp_and_safe_revision_constraints_round_trip(value: dict) -> None:
    assert Schema.model_validate(value).model_dump(exclude_unset=True) == value


def test_circular_references_fail_explicitly() -> None:
    schemas = {"Loop": Schema.model_validate({"$ref": "#/components/schemas/Loop"})}
    with pytest.raises(ValueError, match="cycle"):
        schema_order(schemas)


def test_recursive_object_fields_are_supported() -> None:
    schemas = {"Tree": Schema.model_validate({"type": "object", "properties": {
        "children": {"type": "array", "items": {"$ref": "#/components/schemas/Tree"}},
    }})}
    assert schema_order(schemas) == ["Tree"]


def test_empty_schema_document_fails(tmp_path: Path) -> None:
    path = tmp_path / "openapi.json"
    path.write_text('{"components": {"schemas": {}}}')
    with pytest.raises(ValueError, match="no schemas"):
        read_schemas(path)


@pytest.mark.parametrize("value", [
    {"type": "integer", "maxLength": 4},
    {"type": "string", "maxItems": 4},
    {"type": "string", "maxLength": -1},
    {"type": "array", "items": {"type": "boolean"}, "maxItems": -1},
])
def test_length_budgets_must_match_their_shape(value: dict) -> None:
    with pytest.raises(ValidationError):
        Schema.model_validate(value)

@pytest.mark.parametrize(("shape", "default"), [
    ({"type": ["string", "null"]}, None),
    ({"oneOf": [{"type": "null"}, {"$ref": "#/components/schemas/Value"}]}, None),
    ({"type": "string"}, "example"),
    ({"type": "object", "additionalProperties": {"type": "boolean"}}, {"enabled": True}),
])
def test_default_is_annotation_not_field_presence(shape: dict, default: object) -> None:
    from capsem_builder.sdkgen.python import render_models as python_models
    from capsem_builder.sdkgen.typescript import render_models as typescript_models
    from capsem_builder.sdkgen.typescript_validation import render_validators

    def models(property_schema: dict) -> dict[str, Schema]:
        return {"Value": Schema(type="string"), "Owner": Schema.model_validate({
            "type": "object", "additionalProperties": False,
            "properties": {"optional": property_schema},
        })}

    plain = models(shape)
    annotated = models({**shape, "default": default})
    assert annotated["Owner"].required == []
    for render in (python_models, typescript_models, render_validators):
        assert render(annotated) == render(plain)


@pytest.mark.parametrize("value", [
    {"type": "string", "default": None},
    {"type": ["string", "null"], "default": object()},
    {"type": ["string", "null"], "default": None, "unknownConstraint": True},
])
def test_defaults_do_not_weaken_supported_schema_constraints(value: dict) -> None:
    with pytest.raises(ValueError):
        Schema.model_validate(value)


def test_single_reference_default_uses_the_named_validator_without_union() -> None:
    from capsem_builder.sdkgen.typescript_validation import expression
    schema = Schema.model_validate({"oneOf": [{"$ref": "#/components/schemas/Direction"}], "default": {"enabled": True}})
    assert expression(schema) == expression(Schema.model_validate({"$ref": "#/components/schemas/Direction"}))

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
    assert set(schema_order(schemas)) == set(original)
    for name, schema in schemas.items():
        assert schema.model_dump(by_alias=True, exclude_unset=True) == original[name]


@pytest.mark.parametrize("value", [
    {"type": "string", "pattern": "[a-z]+"},
    {"type": "string", "format": "date-time"},
    {"type": "string", "format": "int32"},
    {"type": "string", "minimum": 0},
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
    {},
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

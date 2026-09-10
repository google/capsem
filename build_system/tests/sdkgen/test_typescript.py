"""TypeScript generation keeps named enums, nullability and reference ownership."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.sdkgen.schema import Schema, read_schemas
from capsem_builder.sdkgen.typescript import render_models, type_name

ROOT = Path(__file__).resolve().parents[3]


def test_every_schema_has_a_compact_deterministic_module() -> None:
    schemas = read_schemas(ROOT / "sdk/specification/openapi.json")
    files = render_models(schemas)
    assert files == render_models(dict(reversed(list(schemas.items()))))
    assert set(files) == {"index.ts", *(name + ".ts" for name in schemas)}
    assert max(len(source.splitlines()) for source in files.values()) <= 300
    for name, schema in schemas.items():
        assert "export " in files[name + ".ts"]
        for dependency in schema.references() - {name}:
            assert f'from "./{dependency}.js"' in files[name + ".ts"]


def test_optional_and_nullable_fields_remain_distinct() -> None:
    schema = Schema.model_validate({"type": "object", "properties": {
        "nullable": {"type": ["string", "null"]},
        "optional": {"type": "boolean"},
        "from": {"type": "string"},
    }, "required": ["nullable", "from"]})
    source = render_models({"Example": schema})["Example.ts"]
    assert '"nullable": string | null;' in source
    assert '"optional"?: boolean;' in source
    assert '"from": string;' in source


def test_enum_values_have_runtime_members() -> None:
    schema = Schema.model_validate({"type": "string", "enum": ["http.request", "import"]})
    source = render_models({"Event": schema})["Event.ts"]
    assert 'export enum Event' in source
    assert 'HTTP_REQUEST = "http.request"' in source
    assert 'IMPORT = "import"' in source


@pytest.mark.parametrize(("value", "expected"), [
    ({"type": "array", "items": {"oneOf": [{"type": "string"}, {"type": "null"}]}},
     "Array<string | null>"),
    ({"type": "object", "additionalProperties": {"type": "boolean"}},
     "Record<string, boolean>"),
    ({"type": "string", "format": "binary"}, "Uint8Array"),
    ({"type": "integer", "format": "int64", "minimum": 0}, "number"),
])
def test_nested_and_binary_types(value: object, expected: str) -> None:
    assert type_name(Schema.model_validate(value)) == expected


@pytest.mark.parametrize("value", [
    {"type": "string", "enum": ["inline"]},
    {"type": "object", "properties": {"x": {"type": "string"}}},
])
def test_inline_complex_types_require_named_schemas(value: object) -> None:
    with pytest.raises(ValueError, match="inline"):
        type_name(Schema.model_validate(value))


def test_colliding_enum_members_are_rejected() -> None:
    schema = Schema.model_validate({"type": "string", "enum": ["http.request", "http_request"]})
    with pytest.raises(ValueError, match="colliding"):
        render_models({"Event": schema})


def test_invalid_schema_identifiers_are_rejected() -> None:
    with pytest.raises(ValueError, match="identifier"):
        render_models({"../Escape": Schema.model_validate({"type": "string"})})

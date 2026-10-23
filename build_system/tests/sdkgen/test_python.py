"""Generated Python types validate real schema shapes, not generator snapshots."""

from __future__ import annotations

import importlib
import json
import sys
from collections.abc import Iterator
from pathlib import Path
from types import ModuleType

import pytest
from capsem_builder.sdkgen.python import render_models, type_name
from capsem_builder.sdkgen.schema import Schema, read_schemas
from pydantic import TypeAdapter, ValidationError

ROOT = Path(__file__).resolve().parents[3]
SCHEMAS = read_schemas(ROOT / "sdk/specification/openapi.json")


@pytest.fixture
def models(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[ModuleType]:
    package = tmp_path / "sdk_generated"
    package.mkdir()
    schemas = {**SCHEMAS, "KeywordRecord": Schema.model_validate({
        "type": "object", "properties": {"from": {"type": "boolean"},
                                           "class": {"type": "string"}},
        "required": ["class"],
    })}
    for name, source in render_models(schemas).items():
        (package / name).write_text(source)
    monkeypatch.syspath_prepend(str(tmp_path))
    try:
        yield importlib.import_module("sdk_generated")
    finally:
        for name in list(sys.modules):
            if name == "sdk_generated" or name.startswith("sdk_generated."):
                del sys.modules[name]


def _sample(schema: Schema) -> object:
    if not (schema.model_fields_set - {"description"}):
        return {"nested": [True, None, {"number": 3}]}
    if schema.ref:
        return _sample(SCHEMAS[schema.ref.rsplit("/", 1)[1]])
    if schema.one_of:
        return _sample(schema.one_of[0])
    if isinstance(schema.type, list) or schema.type == "null":
        return None
    if schema.enum:
        return schema.enum[0]
    if schema.type == "object":
        if isinstance(schema.additional_properties, Schema):
            return {"key": _sample(schema.additional_properties)}
        return {name: _sample(schema.properties[name]) for name in schema.required}
    if schema.type == "array":
        return []
    assert schema.type is not None
    if schema.format == "ipv4":
        return "10.128.0.2"
    if schema.format == "uuid":
        return "fc120782-2272-4b3d-a2e6-a26ee2f8ab61"
    if schema.format == "date-time":
        return "2026-10-14T12:34:56.123456789+01:30"
    if schema.pattern == "^[0-9a-f]{64}$":
        return "af" * 32
    if schema.type == "integer" and schema.minimum is not None:
        return max(0, schema.minimum)
    return {"string": "value", "integer": 0, "number": 0.5, "boolean": True}[schema.type]


@pytest.mark.parametrize("name", sorted(SCHEMAS))
def test_every_exported_type_validates_a_wire_value(name: str, models: ModuleType) -> None:
    value = _sample(SCHEMAS[name])
    adapter = TypeAdapter(getattr(models, name))
    parsed = adapter.validate_json(json.dumps(value))
    assert json.loads(adapter.dump_json(parsed, by_alias=True, exclude_unset=True)) == value


def test_enums_are_runtime_enums_and_reject_magic_values(models: ModuleType) -> None:
    assert models.HostLogSource.SERVICE.value == "service"
    assert models.CredentialEventType.HTTP_REQUEST.value == "http.request"
    with pytest.raises(ValidationError):
        TypeAdapter(models.HostLogSource).validate_json('"unknown"')


def test_nullable_recursive_file_tree_and_large_unsigned_values(models: ModuleType) -> None:
    leaf = {"name": "large.bin", "path": "/large.bin", "type": "file",
            "size": 4294967296, "mtime": 0, "children": None}
    tree = {**leaf, "name": "folder", "type": "directory", "children": [leaf]}
    parsed = models.FileListEntry.model_validate_json(json.dumps(tree))
    assert parsed.type is models.FileEntryType.DIRECTORY
    assert parsed.children[0].size == 4294967296
    assert parsed.children[0].children is None
    with pytest.raises(ValidationError):
        models.FileListEntry.model_validate_json(json.dumps({**leaf, "size": -1}))


def test_optional_nonnullable_is_omitted_and_rejects_explicit_null(models: ModuleType) -> None:
    request = models.UpdateApplyRequest()
    assert request.confirmed is None
    assert request.model_dump(exclude_unset=True) == {}
    assert models.UpdateApplyRequest(confirmed=False).model_dump(exclude_unset=True) == {
        "confirmed": False,
    }
    for value in ({"confirmed": None}, {"confirmed": "true"}, {"unexpected": True}):
        with pytest.raises(ValidationError):
            models.UpdateApplyRequest.model_validate_json(json.dumps(value))


def test_union_preserves_numeric_and_enum_alternatives(models: ModuleType) -> None:
    adapter = TypeAdapter(models.TimelineStatus)
    assert adapter.validate_json("403") == 403
    assert adapter.validate_json('"denied"') is models.ToolDecision.DENIED
    with pytest.raises(ValidationError):
        adapter.validate_json('"403"')


def test_keyword_aliases_preserve_wire_names_and_null_validation(models: ModuleType) -> None:
    parsed = models.KeywordRecord.model_validate_json('{"class": "value", "from": false}')
    assert parsed.class_ == "value"
    assert parsed.from_ is False
    assert parsed.model_dump(by_alias=True, exclude_unset=True) == {"class": "value", "from": False}
    for value in ({"class": "value", "from": None}, {"class_": "value", "from_": None}):
        with pytest.raises(ValidationError):
            models.KeywordRecord.model_validate(value)


def test_inventory_schema_preserves_base_model_method_and_wire_name(models: ModuleType) -> None:
    value = {"schema": "capsem.packages.v1", "subject": "sha256:abc", "architecture": "arm64", "packages": [], "tools": []}
    parsed = models.ImagePackageInventory.model_validate(value)
    assert parsed.schema_ == value["schema"]
    assert callable(type(parsed).schema)
    assert parsed.model_dump(by_alias=True) == value


def test_generation_is_deterministic_and_compact() -> None:
    first = render_models(SCHEMAS)
    assert first == render_models(dict(reversed(list(SCHEMAS.items()))))
    assert max(len(source.splitlines()) for source in first.values()) <= 300
    assert len(first) == len(SCHEMAS) + 2 + (len(SCHEMAS) + 79) // 80


def test_ipv4_is_a_validated_address_value() -> None:
    schema = Schema.model_validate({"type": "string", "format": "ipv4"})
    assert type_name(schema) == "IPv4Address"


def test_connection_datetime_and_revision_validation_preserve_wire_values(models: ModuleType) -> None:
    adapter = TypeAdapter(models.ConnectionSummary)
    value = _sample(SCHEMAS["ConnectionSummary"])
    assert isinstance(value, dict), "ConnectionSummary samples must be wire objects"
    value["revision"] = 9007199254740991
    parsed = adapter.validate_json(json.dumps(value))
    assert json.loads(adapter.dump_json(parsed)) == value
    for timestamp in ("2026-02-30T12:00:00Z", "2026-10-14T12:00:00", "123", 123):
        with pytest.raises(ValidationError):
            adapter.validate_json(json.dumps({**value, "updated_at": timestamp}))
    with pytest.raises(ValidationError):
        adapter.validate_json(json.dumps({**value, "revision": 9007199254740992}))


@pytest.mark.parametrize("value", [
    {"type": "string", "enum": ["first", "second"]},
    {"type": "object", "properties": {"field": {"type": "string"}}},
])
def test_inline_complex_types_require_named_contracts(value: object) -> None:
    with pytest.raises(ValueError, match="inline"):
        type_name(Schema.model_validate(value))


def test_colliding_enum_members_fail_instead_of_overwriting() -> None:
    schema = Schema.model_validate({"type": "string", "enum": ["http.request", "http_request"]})
    with pytest.raises(ValueError, match="colliding"):
        render_models({"Event": schema})


def test_generated_text_and_collection_budgets_reject_oversize_values() -> None:
    from typing import Annotated

    from pydantic import Field, StrictStr

    namespace = {"Annotated": Annotated, "Field": Field, "StrictStr": StrictStr}
    for schema, valid, oversized in [
        ({"type": "string", "maxLength": 4}, "four", "large"),
        ({"type": "array", "items": {"type": "string"}, "maxItems": 2}, ["a", "b"], ["a", "b", "c"]),
    ]:
        adapter = TypeAdapter(eval(type_name(Schema.model_validate(schema)), namespace))
        assert adapter.validate_python(valid) == valid
        with pytest.raises(ValidationError):
            adapter.validate_python(oversized)

"""Rust generic inline objects retain named typed models, with no fallback."""
from __future__ import annotations

import json
from pathlib import Path

import pytest
from capsem_builder.sdkgen.schema import read_schemas
from capsem_builder.sdkgen.typescript import render_models


def specification(tmp_path: Path, schemas: dict) -> Path:
    path = tmp_path / "openapi.json"
    path.write_text(json.dumps({"components": {"schemas": schemas}}))
    return path


def test_inline_array_objects_gain_stable_typed_names(tmp_path: Path) -> None:
    path = specification(tmp_path, {"Envelope_Generic": {"type": "object", "properties": {
        "items": {"type": "array", "items": {"type": "object", "additionalProperties": False,
            "required": ["digest"], "properties": {"digest": {"type": "string"}}}},
    }}})
    schemas = read_schemas(path)
    item = schemas["Envelope_Generic"].properties["items"].items
    assert item is not None and item.ref == "#/components/schemas/Envelope_GenericItemsItem"
    assert schemas["Envelope_GenericItemsItem"].required == ["digest"]
    assert "Envelope_Generic.ts" in render_models(schemas)
    assert read_schemas(path) == schemas


@pytest.mark.parametrize("key", ["a-b", "../escape", "", "a.b"])
def test_inline_model_paths_refuse_ambiguous_or_unsafe_identifiers(tmp_path: Path, key: str) -> None:
    path = specification(tmp_path, {"Root": {"type": "object", "properties": {
        key: {"type": "object", "properties": {"value": {"type": "string"}}},
    }}})
    with pytest.raises(ValueError):
        read_schemas(path)


def test_inline_name_collision_never_overwrites_an_existing_model(tmp_path: Path) -> None:
    path = specification(tmp_path, {
        "Root": {"type": "object", "properties": {"nested": {"type": "object", "properties": {"x": {"type": "string"}}}}},
        "RootNested": {"type": "string"},
    })
    with pytest.raises(ValueError, match="collision"):
        read_schemas(path)


def test_inline_closed_enums_remain_named_and_keep_their_annotation(tmp_path: Path) -> None:
    path = specification(tmp_path, {"Options": {"type": "object", "properties": {
        "mode": {"type": "string", "enum": ["disable", "allow"], "default": "disable"},
    }}})
    schemas = read_schemas(path)
    assert schemas["Options"].properties["mode"].ref == "#/components/schemas/OptionsMode"
    assert schemas["OptionsMode"].enum == ["disable", "allow"]
    assert schemas["OptionsMode"].default_value == "disable"
    assert "export enum OptionsMode" in render_models(schemas)["OptionsMode.ts"]

"""Tagged Rust variants remain named, validated SDK models, not unknown data."""

import json

import pytest
from capsem_builder.sdkgen.python import render_models as python_models
from capsem_builder.sdkgen.schema import read_schemas
from capsem_builder.sdkgen.typescript import render_models as ts_models
from capsem_builder.sdkgen.typescript_validation import render_validators


def variants(tmp_path, extra=None):
    schemas = {"Control": {"oneOf": [
        {"type": "object", "required": ["type", "cols"], "properties": {
            "type": {"type": "string", "enum": ["resize"]},
            "cols": {"type": "integer", "minimum": 1, "maximum": 512},
        }},
        {"type": "object", "required": ["type"], "properties": {
            "type": {"type": "string", "enum": ["close_stdin"]},
        }},
    ]}, **(extra or {})}
    path = tmp_path / "schema.json"
    path.write_text(json.dumps({"components": {"schemas": schemas}}))
    return path


def test_tagged_union_models_have_stable_names_and_literal_discriminators(tmp_path):
    schemas = read_schemas(variants(tmp_path))
    assert {"Control", "ControlResize", "ControlCloseStdin"} == schemas.keys()
    ts = ts_models(schemas)
    assert '"type": "resize"' in ts["ControlResize.ts"]
    assert "ControlResize | ControlCloseStdin" in ts["Control.ts"]
    py = python_models(schemas)
    assert 'Literal["resize"]' in py["control_resize.py"]
    assert "from typing import Annotated, Literal" in py["control_resize.py"]
    validators = render_validators(schemas)
    assert 'z.literal("resize")' in validators["ControlResize.ts"]
    assert ".min(1).max(512)" in validators["ControlResize.ts"]


def test_variant_model_names_cannot_overwrite_an_existing_model(tmp_path):
    path = variants(tmp_path, {"ControlResize": {"type": "string"}})
    with pytest.raises(ValueError, match="collision"):
        read_schemas(path)


def test_untagged_inline_union_objects_are_still_refused(tmp_path):
    path = variants(tmp_path)
    document = json.loads(path.read_text())
    del document["components"]["schemas"]["Control"]["oneOf"][0]["properties"]["type"]
    document["components"]["schemas"]["Control"]["oneOf"][0]["required"] = ["cols"]
    path.write_text(json.dumps(document))
    with pytest.raises(ValueError, match="discriminator"):
        read_schemas(path)


def test_reordering_variants_preserves_their_model_identity(tmp_path):
    path = variants(tmp_path)
    first = read_schemas(path)
    document = json.loads(path.read_text())
    document["components"]["schemas"]["Control"]["oneOf"].reverse()
    path.write_text(json.dumps(document))
    second = read_schemas(path)
    for name in ("ControlResize", "ControlCloseStdin"):
        assert first[name] == second[name]


def test_duplicate_variant_tags_are_rejected(tmp_path):
    path = variants(tmp_path)
    document = json.loads(path.read_text())
    members = document["components"]["schemas"]["Control"]["oneOf"]
    members.append(members[0])
    path.write_text(json.dumps(document))
    with pytest.raises(ValueError, match="collision"):
        read_schemas(path)

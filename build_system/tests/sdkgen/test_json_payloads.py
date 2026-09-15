"""Unconstrained JSON stays JSON; it never becomes Any or a serialized string."""

from capsem_builder.sdkgen.python import render_models as python_models
from capsem_builder.sdkgen.schema import Schema
from capsem_builder.sdkgen.typescript import render_models as typescript_models
from capsem_builder.sdkgen.typescript_validation import render_validators


def test_payload_json_schema_preserves_recursive_json_types() -> None:
    schemas = {"Payload": Schema.model_validate({"type": "object", "required": ["value"],
               "properties": {"value": {}}})}
    python = python_models(schemas)["payload.py"]
    assert "JsonValue" in python
    assert "value: JsonValue" in python
    typescript = typescript_models(schemas)["Payload.ts"]
    assert "JSONType" in typescript
    assert '"value": JSONType' in typescript
    assert '"value": z.json()' in render_validators(schemas)["Payload.ts"]

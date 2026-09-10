"""Exercise the installed SDK's complete OpenAPI surface, including generation."""

from __future__ import annotations

import asyncio
import json
import re
from enum import StrEnum
from pathlib import Path
from typing import Any

import pytest
from capsem import _operations, models
from capsem._transport import Transport
from pydantic import TypeAdapter, ValidationError

from .test_transport import gateway

SPEC = json.loads((Path(__file__).resolve().parents[2] / "specification/openapi.json").read_text())
SCHEMAS = SPEC["components"]["schemas"]
OPERATIONS = [operation for methods in SPEC["paths"].values() for operation in methods.values()]


def sample(schema: dict[str, Any]) -> object:
    if not (schema.keys() - {"description"}):
        return {"nested": [True, None, {"number": 3}]}
    if "$ref" in schema:
        return sample(SCHEMAS[schema["$ref"].rsplit("/", 1)[1]])
    if "oneOf" in schema:
        return sample(schema["oneOf"][0])
    if "enum" in schema:
        return schema["enum"][0]
    kind = schema["type"]
    if isinstance(kind, list) or kind == "null":
        return None
    if kind == "object":
        if isinstance(schema.get("additionalProperties"), dict):
            return {"key": sample(schema["additionalProperties"])}
        return {key: sample(schema["properties"][key]) for key in schema.get("required", [])}
    if kind == "array":
        return []
    return {"string": "value", "integer": 0, "number": 0.5, "boolean": True}[kind]


@pytest.mark.parametrize("name", sorted(SCHEMAS))
def test_every_public_model_roundtrips_a_wire_value(name: str) -> None:
    value = sample(SCHEMAS[name])
    adapter = TypeAdapter(getattr(models, name))
    parsed = adapter.validate_json(json.dumps(value))
    assert json.loads(adapter.dump_json(parsed, by_alias=True, exclude_unset=True)) == value


@pytest.mark.parametrize("name", [name for name, schema in SCHEMAS.items() if "enum" in schema])
def test_named_enums_reject_unknown_values(name: str) -> None:
    model = getattr(models, name)
    assert issubclass(model, StrEnum)
    with pytest.raises(ValidationError):
        TypeAdapter(model).validate_json('"unknown-enum-value"')


def test_optional_nonnullable_fields_cannot_be_explicitly_null() -> None:
    checked = 0
    for name in SCHEMAS:
        model = getattr(models, name)
        for key in getattr(model, "nonnullable_optional", ()):
            value = sample(SCHEMAS[name])
            assert isinstance(value, dict)
            with pytest.raises(ValidationError):
                TypeAdapter(model).validate_json(json.dumps({**value, key: None}))
            checked += 1
    assert checked > 0


@pytest.mark.parametrize("operation", OPERATIONS, ids=lambda operation: operation["operationId"])
def test_each_packaged_operation_uses_http_and_returns_typed_data(operation: dict[str, Any]) -> None:
    status = "200" if "200" in operation["responses"] else "202"
    content = operation["responses"][status]["content"]
    binary = "application/octet-stream" in content
    expected = b"\x00\xff" if binary else sample(content["application/json"]["schema"])
    response = expected if isinstance(expected, bytes) else json.dumps(expected).encode()
    arguments = {parameter["name"]: sample(parameter["schema"])
                 for parameter in operation.get("parameters", []) if parameter["required"]}
    if "requestBody" in operation:
        content = operation["requestBody"]["content"]
        if "application/octet-stream" in content:
            arguments["body"] = b"\x00\xff"
        else:
            schema = content["application/json"]["schema"]
            model = getattr(models, schema["$ref"].rsplit("/", 1)[1])
            arguments["body"] = model.model_validate_json(json.dumps(sample(schema)))

    async def run() -> None:
        async with gateway(response) as (url, received), Transport(url, "token") as transport:
            name = re.sub(r"(?<!^)(?=[A-Z])", "_", operation["operationId"]).lower()
            result = await getattr(_operations, name)(transport, **arguments)
            actual = result if isinstance(result, bytes) else json.loads(
                TypeAdapter(type(result)).dump_json(result, by_alias=True, exclude_unset=True),
            )
            assert actual == expected
            assert len(received) == 1
            assert received[0][3]["Authorization"] == "Bearer token"
    asyncio.run(run())

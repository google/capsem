"""Rust generation is deterministic, bounded, typed and fails on unsupported wire shapes."""

from pathlib import Path

import pytest
from capsem_builder.sdkgen.operations import Route, read_operations
from capsem_builder.sdkgen.rust_operations import render_operations, type_name, wire_value
from capsem_builder.sdkgen.schema import Schema

SPEC = Path(__file__).resolve().parents[3] / "sdk/specification/openapi.json"


def test_all_routes_are_generated_in_small_modules() -> None:
    routes = read_operations(SPEC)
    sources = render_operations(routes)
    assert len(sources) == len(routes) + 1
    assert max(len(source.splitlines()) for source in sources.values()) < 150
    assert sources == render_operations(list(reversed(routes)))


@pytest.mark.parametrize(("schema", "expected"), [
    ({"type": "integer", "format": "int32"}, "i32"),
    ({"type": "integer", "minimum": 0}, "u64"),
    ({"type": "boolean"}, "bool"),
])
def test_primitive_types(schema: dict, expected: str) -> None:
    assert type_name(Schema.model_validate(schema)) == expected


@pytest.mark.parametrize("schema", [{"type": "number"}, {"type": "integer", "minimum": 2},
                                    {"type": "string", "enum": ["A"]}])
def test_unhandled_types_fail_closed(schema: dict) -> None:
    with pytest.raises(ValueError, match="unsupported Rust operation schema"):
        type_name(Schema.model_validate(schema))


def test_unhandled_query_encoding_fails_closed() -> None:
    with pytest.raises(ValueError, match="wire parameter"):
        wire_value(Schema(type="number"), "value")


def test_collisions_and_reserved_identifiers_fail_closed() -> None:
    route = read_operations(SPEC)[0]
    with pytest.raises(ValueError, match="colliding"):
        render_operations([route, route])
    invalid = route.operation.model_copy(update={"operation_id": "type"})
    with pytest.raises(ValueError, match="operation"):
        render_operations([Route(route.path, route.method, invalid)])
    parameter = route.operation.parameters[0].model_copy(update={"name": "type"})
    invalid = route.operation.model_copy(update={"parameters": [parameter]})
    with pytest.raises(ValueError, match="parameter"):
        render_operations([Route(route.path, route.method, invalid)])


def test_request_body_cannot_overwrite_a_parameter() -> None:
    route = next(route for route in read_operations(SPEC) if route.operation.request_body and route.operation.parameters)
    parameter = route.operation.parameters[0].model_copy(update={"name": "body"})
    invalid = route.operation.model_copy(update={"parameters": [parameter]})
    with pytest.raises(ValueError, match="body"):
        render_operations([Route(route.path, route.method, invalid)])

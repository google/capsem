"""TypeScript operation generation owns every documented HTTP operation."""

from pathlib import Path

import pytest
from capsem_builder.sdkgen.operations import Route, read_operations
from capsem_builder.sdkgen.typescript_operations import render_operations

SPEC = Path(__file__).resolve().parents[3] / "sdk/specification/openapi.json"


def test_all_routes_have_a_compact_module_and_public_export() -> None:
    routes = read_operations(SPEC)
    sources = render_operations(routes)
    assert set(sources) == {"index.ts", *(route.operation.operation_id + ".ts" for route in routes)}
    assert max(len(source.splitlines()) for source in sources.values()) <= 300
    assert sources == render_operations(list(reversed(routes)))


def test_operation_identifiers_cannot_collide_or_be_reserved() -> None:
    route = read_operations(SPEC)[0]
    with pytest.raises(ValueError, match="colliding"):
        render_operations([route, route])
    invalid = route.operation.model_copy(update={"operation_id": "class"})
    with pytest.raises(ValueError, match="identifier"):
        render_operations([Route(route.path, route.method, invalid)])


def test_request_body_cannot_overwrite_a_parameter() -> None:
    route = next(route for route in read_operations(SPEC) if route.operation.request_body and route.operation.parameters)
    parameter = route.operation.parameters[0].model_copy(update={"name": "body"})
    invalid = route.operation.model_copy(update={"parameters": [parameter]})
    with pytest.raises(ValueError, match="body"):
        render_operations([Route(route.path, route.method, invalid)])

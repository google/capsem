"""Every generated Python operation executes its documented HTTP contract."""

from __future__ import annotations

import asyncio
import importlib
import json
import sys
from collections.abc import Iterator
from pathlib import Path
from types import ModuleType
from urllib.parse import quote

import pytest
from aiohttp import web
from capsem_builder.sdkgen.operations import Route, read_operations
from capsem_builder.sdkgen.python import module_name, render_models, type_name
from capsem_builder.sdkgen.python_operations import render_operations
from capsem_builder.sdkgen.schema import Schema, read_schemas
from pydantic import TypeAdapter, ValidationError

ROOT = Path(__file__).resolve().parents[3]
SPEC = ROOT / "sdk/specification/openapi.json"
SCHEMAS = read_schemas(SPEC)
ROUTES = read_operations(SPEC)


def sample(schema: Schema) -> object:
    if schema.ref:
        return sample(SCHEMAS[schema.ref.rsplit("/", 1)[1]])
    if schema.one_of:
        return sample(schema.one_of[0])
    if schema.enum:
        return schema.enum[0]
    if isinstance(schema.type, list) or schema.type == "null":
        return None
    if schema.type == "object":
        return {key: sample(schema.properties[key]) for key in schema.required}
    if schema.type == "array":
        return []
    assert schema.type is not None
    return {"string": "value", "integer": 0, "number": 0.5, "boolean": True}[schema.type]


@pytest.fixture
def package(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[ModuleType]:
    root = tmp_path / "generated_client"
    root.mkdir()
    (root / "__init__.py").write_text("")
    (root / "_transport.py").write_text((ROOT / "sdk/python/capsem/_transport.py").read_text())
    for directory, files in (("models", render_models(SCHEMAS)), ("operations", render_operations(ROUTES))):
        (root / directory).mkdir()
        for name, source in files.items():
            (root / directory / name).write_text(source)
    monkeypatch.syspath_prepend(str(tmp_path))
    try:
        module = importlib.import_module("generated_client")
        importlib.import_module("generated_client.operations")
        yield module
    finally:
        for name in list(sys.modules):
            if name == "generated_client" or name.startswith("generated_client."):
                del sys.modules[name]


@pytest.mark.parametrize(("route", "outcome"), [
    pytest.param(route, outcome, id=f"{route.operation.operation_id}-{outcome}")
    for route in ROUTES for outcome in ("success", "http_error", "malformed")
    if outcome != "malformed" or route.operation.success.media_type == "application/json"
])
def test_operation_matches_the_wire_contract(route: Route, outcome: str, package: ModuleType) -> None:
    operation = route.operation
    response = operation.success
    expected = b"\x00\xff" if response.media_type == "application/octet-stream" else sample(response.schema)
    arguments = {}
    expected_path = route.path
    expected_query = {}
    for parameter in operation.parameters:
        value = sample(parameter.schema_)
        if parameter.schema_.type == "array":
            assert parameter.schema_.items is not None
            value = [sample(parameter.schema_.items)] * 2
        if parameter.name == "id":
            value = "a/b .. café"
        arguments[parameter.name] = value
        if parameter.location == "path":
            expected_path = expected_path.replace("{" + parameter.name + "}", quote(str(value), safe="").replace(".", "%2E"))
        else:
            expected_query[parameter.name] = ",".join(map(str, value)) if isinstance(value, list) else str(value)
    expected_body = b""
    if operation.request_body:
        if operation.request_body.media_type == "application/octet-stream":
            arguments["body"] = expected_body = b"\x00\xff"
        else:
            payload = sample(operation.request_body.schema)
            model = getattr(package.models, type_name(operation.request_body.schema))
            arguments["body"] = model.model_validate_json(json.dumps(payload))
            expected_body = arguments["body"].model_dump_json(by_alias=True, exclude_unset=True).encode()

    async def run() -> None:
        received = []

        async def handle(request: web.Request) -> web.Response:
            received.append((request.method, request.raw_path.split("?")[0], dict(request.query),
                             await request.read(), dict(request.headers)))
            if outcome == "http_error":
                return web.Response(status=403, text="denied")
            if outcome == "malformed":
                return web.Response(body=b"{")
            status = int(operation.success_status)
            return web.Response(body=expected, status=status) if isinstance(expected, bytes) else web.json_response(expected, status=status)

        app = web.Application()
        app.router.add_route("*", "/{tail:.*}", handle)
        runner = web.AppRunner(app)
        await runner.setup()
        try:
            await web.TCPSite(runner, "127.0.0.1", 0).start()
            async with package._transport.Transport(f"http://127.0.0.1:{runner.addresses[0][1]}", "secret") as transport:
                call = getattr(package.operations, module_name(operation.operation_id))
                if outcome == "success":
                    result = await call(transport, **arguments)
                    actual = result if isinstance(result, bytes) else json.loads(TypeAdapter(type(result)).dump_json(result, by_alias=True, exclude_unset=True))
                    assert actual == expected
                else:
                    error = package._transport.HttpError if outcome == "http_error" else ValidationError
                    with pytest.raises(error) as caught:
                        await call(transport, **arguments)
                    if outcome == "http_error":
                        assert vars(caught.value)["status"] == 403
                        assert vars(caught.value)["body"] == "denied"
            assert len(received) == 1
            method, path, query, body, headers = received[0]
            assert (method, path, query, body) == (route.method, expected_path, expected_query, expected_body)
            assert headers["Authorization"] == "Bearer secret"
            assert headers["Accept"] == response.media_type
            if operation.request_body:
                assert headers["Content-Type"] == operation.request_body.media_type
        finally:
            await runner.cleanup()

    asyncio.run(run())


def test_invalid_input_fails_before_network_io(package: ModuleType) -> None:
    async def run() -> None:
        async with package._transport.Transport("http://127.0.0.1:1", "token") as transport:
            with pytest.raises(ValidationError):
                await package.operations.get_vm_info(transport, id=12)
            with pytest.raises(ValidationError):
                await package.operations.get_hypervisor_logs(transport, name="unknown")
            with pytest.raises(ValidationError):
                await package.operations.get_vm_changes(transport, id="vm", checkpoint="cp-10", limit=-1)
    asyncio.run(run())


def test_invalid_identifiers_and_collisions_fail_generation() -> None:
    route = ROUTES[0]
    invalid = route.operation.model_copy(update={"operation_id": "class"})
    with pytest.raises(ValueError, match="identifier"):
        render_operations([Route(route.path, route.method, invalid)])
    with pytest.raises(ValueError, match="colliding"):
        render_operations([route, route])
    parameter = route.operation.parameters[0].model_copy(update={"name": "transport"})
    invalid = route.operation.model_copy(update={"parameters": [parameter]})
    with pytest.raises(ValueError, match="parameter"):
        render_operations([Route(route.path, route.method, invalid)])

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

import pytest
from aiohttp import web
from capsem._transport import HttpError, MediaType, Method, Transport
from pydantic import BaseModel


@asynccontextmanager
async def gateway() -> AsyncIterator[tuple[str, list[tuple[str, str, bytes, dict[str, str]]]]]:
    received: list[tuple[str, str, bytes, dict[str, str]]] = []

    async def handle(request: web.Request) -> web.Response:
        payload = await request.read()
        received.append((request.method, request.raw_path, payload, dict(request.headers)))
        if request.path == "/error":
            return web.Response(status=403, text="denied")
        if request.path == "/redirect":
            return web.Response(status=307, headers={"Location": "/unexpected"})
        if request.path == "/wait":
            await asyncio.sleep(0.1)
        return web.Response(body=payload or b'{"success":true}', content_type="application/octet-stream")

    app = web.Application()
    app.router.add_route("*", "/{tail:.*}", handle)
    runner = web.AppRunner(app, shutdown_timeout=0.2)
    await runner.setup()
    try:
        await web.TCPSite(runner, "127.0.0.1", 0).start()
        port = runner.addresses[0][1]
        yield f"http://127.0.0.1:{port}", received
    finally:
        await runner.cleanup()


def test_wire_encoding_auth_json_and_binary() -> None:
    class Payload(BaseModel):
        command: str
        optional: str | None = None

    async def run() -> None:
        async with gateway() as (url, received), Transport(url, "test-token") as client:
            result = await client.request(
                Method.POST, "/vms/{id}/exec", path_parameters={"id": "a/b .. café"},
                query={"path": "/with spaces/+plus", "layers": ["exec", "tool"],
                       "enabled": False, "limit": 7, "ignored": None},
                body=Payload(command="echo hello"),
            )
            assert result == b'{"command":"echo hello"}'
            method, path, body, headers = received[0]
            assert method == "POST"
            assert path == "/vms/a%2Fb%20%2E%2E%20caf%C3%A9/exec?path=%2Fwith+spaces%2F%2Bplus&layers=exec%2Ctool&enabled=false&limit=7"
            assert headers["Authorization"] == "Bearer test-token"
            assert headers["Accept"] == headers["Content-Type"] == "application/json"
            assert b"optional" not in body
            binary = b"\x00\xff\n"
            assert await client.request(Method.POST, "/file", body=binary, accept=MediaType.BINARY) == binary
            assert received[1][3]["Accept"] == received[1][3]["Content-Type"] == "application/octet-stream"
            assert await client.request(Method.GET, "/status") == b'{"success":true}'
            assert len(received) == 3
        await client.close()
        with pytest.raises(RuntimeError, match="closed"):
            await client.request(Method.GET, "/status")

    asyncio.run(run())


@pytest.mark.parametrize(("path", "status", "body"), [("/error", 403, "denied"), ("/redirect", 307, "")])
def test_http_errors_preserve_status_and_never_replay_mutations(path: str, status: int, body: str) -> None:
    async def run() -> None:
        async with gateway() as (url, received), Transport(url, "token") as client:
            with pytest.raises(HttpError) as failure:
                await client.request(Method.POST, path)
            assert failure.value.status == status
            assert failure.value.body == body
            assert "token" not in str(failure.value)
            assert len(received) == 1

    asyncio.run(run())


def test_timeout_is_bounded_and_does_not_retry() -> None:
    async def run() -> None:
        async with gateway() as (url, received), Transport(url, "token", timeout=0.02) as client:
            with pytest.raises(TimeoutError):
                await client.request(Method.POST, "/wait")
            assert len(received) == 1

    asyncio.run(run())


@pytest.mark.parametrize("url", ["unix:///tmp/service.sock", "ftp://localhost", "http://", "http://user:pass@localhost",
                                  "http://localhost?query=x", "http://localhost#fragment"])
def test_invalid_gateway_urls_are_rejected(url: str) -> None:
    with pytest.raises(ValueError, match="gateway URL"):
        Transport(url, "token")


@pytest.mark.parametrize("token", ["", "a\nb", "a\rb"])
def test_invalid_tokens_are_rejected(token: str) -> None:
    with pytest.raises(ValueError, match="token"):
        Transport("http://localhost", token)


@pytest.mark.parametrize("timeout", [0, -1, float("inf"), float("nan")])
def test_invalid_timeouts_are_rejected(timeout: float) -> None:
    with pytest.raises(ValueError, match="timeout"):
        Transport("http://localhost", "token", timeout=timeout)


def test_invalid_operation_paths_fail_before_network_io() -> None:
    async def run() -> None:
        async with Transport("http://127.0.0.1:1", "token") as client:
            for path in ("relative", "/vms/{id}/info", "/status?query=x", "/status#fragment"):
                with pytest.raises(ValueError, match="path"):
                    await client.request(Method.GET, path)
    asyncio.run(run())

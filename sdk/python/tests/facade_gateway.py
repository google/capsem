"""Stateful HTTP fixture for public SDK lifecycle and resource interactions."""

from __future__ import annotations

import asyncio
import json
import re
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from dataclasses import dataclass, field
from typing import Any

from aiohttp import web

from .test_contract import SCHEMAS, SPEC, sample


@dataclass
class GatewayState:
    requests: list[tuple[str, str, bytes]] = field(default_factory=list)
    files: dict[str, bytes] = field(default_factory=dict)
    names: list[str] = field(default_factory=lambda: ["named"])
    exec_entered: asyncio.Event = field(default_factory=asyncio.Event)
    exec_release: asyncio.Event = field(default_factory=asyncio.Event)
    wait_for_exec: bool = False


def response_model(schema_name: str, **fields: Any) -> dict[str, Any]:
    value = sample(SCHEMAS[schema_name])
    assert isinstance(value, dict)
    return {**value, **fields}


@asynccontextmanager
async def gateway() -> AsyncIterator[tuple[str, GatewayState]]:
    state = GatewayState()

    async def handle(request: web.Request) -> web.Response:
        body = await request.read()
        state.requests.append((request.method, request.raw_path, body))
        assert request.headers["Authorization"] == "Bearer token"
        if request.path == "/vms/list":
            return web.json_response({"sandboxes": [response_model("SandboxInfo", id=f"vm-{index}", name=name)
                                                    for index, name in enumerate(state.names)]})
        if request.path == "/vms/create":
            payload = json.loads(body)
            return web.json_response(response_model("ProvisionResponse", id="created-id", name=payload["name"] or "temporary"))
        if request.path.endswith("/fork"):
            return web.json_response(response_model("ForkResponse", id="forked-id", name=json.loads(body)["name"]))
        if request.path.endswith("/exec"):
            state.exec_entered.set()
            if state.wait_for_exec:
                await state.exec_release.wait()
        if request.path.endswith("/files/content"):
            path = request.query["path"]
            if request.method == "GET":
                if path not in state.files:
                    return web.Response(status=404, text="file not found")
                return web.Response(body=state.files[path])
            state.files[path] = body
        for path, methods in SPEC["paths"].items():
            if re.fullmatch(re.sub(r"\{\w+\}", "[^/]+", path), request.path):
                operation = methods[request.method.lower()]
                return web.json_response(sample(operation["responses"]["200"]["content"]["application/json"]["schema"]))
        raise AssertionError(f"unhandled SDK request {request.method} {request.path}")

    app = web.Application()
    app.router.add_route("*", "/{tail:.*}", handle)
    runner = web.AppRunner(app, shutdown_timeout=0.2)
    await runner.setup()
    try:
        await web.TCPSite(runner, "127.0.0.1", 0).start()
        yield f"http://127.0.0.1:{runner.addresses[0][1]}", state
    finally:
        state.exec_release.set()
        await runner.cleanup()

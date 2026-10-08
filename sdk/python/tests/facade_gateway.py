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
    exec_status: int | None = None
    container_states: list[str] = field(default_factory=lambda: ["running"])
    preview_session_status: int | None = None
    delays: dict[str, float] = field(default_factory=dict)
    create_error: tuple[int, str] | None = None
    sandboxes: dict[str, str] = field(default_factory=dict)
    created_vms: dict[str, tuple[str, bool]] = field(default_factory=dict)
    sandbox_labels: dict[str, dict[str, str] | None] = field(default_factory=dict)


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
        if request.path in state.delays:
            await asyncio.sleep(state.delays[request.path])
        if request.path == "/vms/list":
            listed = [
                response_model("SandboxInfo", id=vm_id, name=name, labels=state.sandbox_labels.get(vm_id))
                for vm_id, name in state.sandboxes.items()
            ] + [
                response_model(
                    "SandboxInfo",
                    id=f"vm-{index}",
                    name=name,
                    labels=state.sandbox_labels.get(f"vm-{index}"),
                )
                for index, name in enumerate(state.names)
            ]
            return web.json_response({"sandboxes": listed})
        if request.path == "/vms/create":
            payload = json.loads(body)
            vm_name = payload["name"] or "temporary"
            if state.create_error is not None:
                status, message = state.create_error
                if status == 504:
                    err_doc = json.loads(message)
                    if err_doc.get("code") == "create_timeout" and isinstance(err_doc.get("vm_id"), str):
                        state.sandboxes[err_doc["vm_id"]] = vm_name
                return web.Response(status=status, text=message, content_type="application/json")
            persistent = bool(payload.get("persistent", bool(payload.get("name") or payload.get("from"))))
            state.created_vms["created-id"] = (vm_name, persistent)
            state.sandbox_labels["created-id"] = payload.get("labels") or None
            return web.json_response(
                response_model(
                    "ProvisionResponse",
                    id="created-id",
                    name=vm_name,
                    persistent=persistent,
                )
            )
        if request.method == "DELETE" and request.path.endswith("/delete"):
            vm_id = request.path.removeprefix("/vms/").removesuffix("/delete")
            state.sandboxes.pop(vm_id, None)
            if vm_id.startswith("vm-") and vm_id[3:].isdigit():
                index = int(vm_id[3:])
                if 0 <= index < len(state.names):
                    state.names.pop(index)
        if request.path.endswith("/info") and request.path.split("/")[2] in state.created_vms:
            vm_id = request.path.split("/")[2]
            vm_name, persistent = state.created_vms[vm_id]
            return web.json_response(
                response_model(
                    "SandboxInfo",
                    id=vm_id,
                    name=vm_name,
                    persistent=persistent,
                    labels=state.sandbox_labels.get(vm_id),
                )
            )
        if request.path == "/networks" and request.method == "POST":
            return web.json_response(response_model("NetworkInfo", name=json.loads(body)["name"]))
        if request.path == "/networks":
            return web.json_response({"networks": [response_model(
                "NetworkInfo", id="net-1", name="team", members=[{
                    "vm_id": "vm-0", "address": "10.0.0.2", "state": "ready", "updated_unix_ms": 1,
                }],
            )]})
        if request.path == "/mcp/servers/list":
            return web.json_response([response_model("McpServerInfoResponse", name="filesystem")])
        if request.path.endswith("/fork"):
            payload = json.loads(body)
            source_id = request.path.split("/")[2]
            fork_id = f"forked-{payload['name']}"
            state.created_vms[fork_id] = (payload["name"], True)
            if "labels" in payload and payload["labels"] is not None:
                state.sandbox_labels[fork_id] = payload["labels"] or None
            else:
                state.sandbox_labels[fork_id] = state.sandbox_labels.get(source_id)
            return web.json_response(response_model("ForkResponse", id="forked-id", name=payload["name"]))
        if request.path.endswith("/exec"):
            if state.exec_status is not None:
                return web.Response(status=state.exec_status, text="exec target unavailable")
            state.exec_entered.set()
            if state.wait_for_exec:
                await state.exec_release.wait()
        if request.path.endswith("/container"):
            state_name = state.container_states.pop(0) if len(state.container_states) > 1 else state.container_states[0]
            return web.json_response(response_model(
                "ContainerStatusResponse", image="docker://busybox:latest",
                state=state_name,
            ))
        if request.path.endswith("/exposures") and request.method == "POST":
            payload = json.loads(body)
            preview = payload["access"] == "http_preview"
            payload["host_port"] = None if preview else (payload["host_port"] or 49152)
            return web.json_response(response_model(
                "ExposureInfo", **payload,
                id="preview-id" if preview else "49152",
            ))
        if request.path.endswith("/preview-session") and state.preview_session_status is not None:
            return web.Response(status=state.preview_session_status, text="preview session refused")
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
                status = "200" if "200" in operation["responses"] else "202"
                return web.json_response(sample(operation["responses"][status]["content"]["application/json"]["schema"]), status=int(status))
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

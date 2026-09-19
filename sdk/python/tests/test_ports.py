"""Port handles: exposure lifecycle, preview sessions, and secret handling."""

from __future__ import annotations

import asyncio
import json
from typing import Any

import pytest
from capsem import HttpError, Hypervisor, Port, models

from .facade_gateway import gateway


def test_ports_hide_wire_exposures_and_infer_the_container_target() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            vm = await hv.create(image="nginx:alpine")
            plain = await vm.ports.open(8080)
            assert isinstance(plain, Port)
            assert plain.guest == 8080 and plain.authenticate is False
            authenticated = await vm.ports.open(3000, authenticate=True)
            assert authenticated.authenticate is True
            assert authenticated.url is not None and authenticated.bootstrap_token is not None
            assert await vm.ports.list() == []
            request_count = len(state.requests)
            raw_id: Any = plain.id
            with pytest.raises(TypeError, match="port must be an object"):
                await vm.ports.close(raw_id)
            assert len(state.requests) == request_count
            assert isinstance(await vm.ports.close(plain), models.VmActionResponse)
            create_bodies = [json.loads(body) for method, path, body in state.requests
                             if method == "POST" and path.endswith("/exposures")]
            assert create_bodies == [
                {"guest_port": 8080, "host_port": 0, "target": "container", "access": "loopback_tcp"},
                {"guest_port": 3000, "host_port": 0, "target": "container", "access": "http_preview"},
            ]
            assert [(method, path.split("?")[0]) for method, path, _ in state.requests] == [
                ("GET", "/status"),
                ("POST", "/vms/create"),
                ("POST", "/vms/created-id/exposures"),
                ("POST", "/vms/created-id/exposures"),
                ("POST", f"/vms/created-id/exposures/{authenticated.id}/preview-session"),
                ("GET", "/vms/created-id/exposures"),
                ("DELETE", f"/vms/created-id/exposures/{plain.id}"),
            ]
    asyncio.run(run())


def test_authenticated_port_closes_its_exposure_when_the_session_fails() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            vm = await hv.create(image="nginx:alpine")
            state.preview_session_status = 503
            with pytest.raises(HttpError) as raised:
                await vm.ports.open(3000, authenticate=True)
            assert raised.value.status == 503
            assert [(method, path) for method, path, _ in state.requests[2:]] == [
                ("POST", "/vms/created-id/exposures"),
                ("POST", "/vms/created-id/exposures/preview-id/preview-session"),
                ("DELETE", "/vms/created-id/exposures/preview-id"),
            ]
    asyncio.run(run())


def test_port_repr_never_prints_the_preview_bootstrap_token() -> None:
    port = Port(
        id="preview-id", guest=3000, host=None, authenticate=True,
        url="http://preview-id.localhost:19223/_capsem/bootstrap",
        bootstrap_token="bootstrap-secret", expires_in_seconds=30,
    )
    for text in (repr(port), str(port), f"{port}", repr([port])):
        assert "bootstrap-secret" not in text
        assert "bootstrap_token=<redacted>" in text
    assert "bootstrap_token=<none>" in repr(Port(id="49152", guest=80, host=49152, authenticate=False))
    assert port.bootstrap_token == "bootstrap-secret"

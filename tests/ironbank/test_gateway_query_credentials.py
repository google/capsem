"""Native browser WebSocket credentials: one operator source, no URI secrets."""

from __future__ import annotations

import asyncio
import secrets
import uuid

import pytest
import websockets
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance
from helpers.service_gateway import ServiceGateway

pytestmark = pytest.mark.integration


def test_query_bearers_are_single_operator_websocket_credentials_and_stay_private():
    service = ServiceInstance(isolated_corp_config=True)
    gateway = ServiceGateway(service)
    capability = secrets.token_hex(32)
    access_id = str(uuid.uuid4())
    operator = None
    try:
        service.start()
        gateway.start()
        operator = TcpHttpClient(gateway.base_url, gateway.token)
        assert operator.call_json("POST", "/agent-access", {
            "request_id": access_id, "ownership_token": capability,
            "scope": {"operations": ["discover"]}, "lease_seconds": 60,
        })[0] == 200
        base = gateway.base_url.replace("http://", "ws://")
        operator_header = {"Authorization": f"Bearer {gateway.token}"}
        agent_header = {"Authorization": f"Bearer {access_id}.{capability}"}

        async def ping(path, headers=None):
            async with websockets.connect(base + path, additional_headers=headers, open_timeout=5, close_timeout=2) as socket:
                pong = await socket.ping(b"credential-rule")
                await asyncio.wait_for(pong, timeout=5)

        async def check():
            await ping(f"/events?token={gateway.token}")
            await ping("/events", operator_header)
            cases = [
                (f"/events?token={gateway.token}&token={gateway.token}", None),
                (f"/events?token={gateway.token}&to%6ben={gateway.token}", None),
                (f"/events?token={gateway.token}", operator_header),
                (f"/events?token={gateway.token}", {"Authorization": "Bearer wrong"}),
                (f"/events?token={gateway.token}", agent_header),
                (f"/events?token={access_id}.{capability}", None),
                (f"/vms/list?token={gateway.token}", None),
            ]
            for path, headers in cases:
                with pytest.raises(websockets.exceptions.InvalidStatus) as refused:
                    await websockets.connect(base + path, additional_headers=headers, open_timeout=5)
                assert refused.value.response.status_code == 401
            # A valid agent header cannot reach the operator event broadcast.
            with pytest.raises(websockets.exceptions.InvalidStatus) as refused:
                await websockets.connect(base + "/events", additional_headers=agent_header, open_timeout=5)
            assert refused.value.response.status_code == 403
            await ping(f"/events?token={gateway.token}&client=browser")

        asyncio.run(check())
        for method in ("GET", "POST"):
            assert operator.call(method, f"/events?token={gateway.token}", use_auth=False)[0] == 401
        assert operator.call_json("GET", "/vms/list")[0] == 200
        service_log = service.stop_and_read_log()
        gateway_log = gateway.stop_and_read_log()
        for secret in (gateway.token, capability, f"{access_id}.{capability}"):
            assert secret not in service_log
            assert secret not in gateway_log
    finally:
        if operator is not None:
            operator.close()
        gateway.stop()
        service.stop()

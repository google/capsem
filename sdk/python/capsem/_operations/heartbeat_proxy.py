"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.proxy_heartbeat_response import ProxyHeartbeatResponse
from ..models.proxy_lease_request import ProxyLeaseRequest


async def heartbeat_proxy(
    transport: Transport,
    *,
    id: StrictStr,
    body: ProxyLeaseRequest,
    request_timeout: float | None = None,
) -> ProxyHeartbeatResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/proxies/{id}/heartbeat',
        path_parameters={'id': id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(ProxyHeartbeatResponse).validate_json(payload)

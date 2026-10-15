"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.create_proxy_request import CreateProxyRequest
from ..models.create_proxy_response import CreateProxyResponse


async def create_proxy(
    transport: Transport,
    *,
    body: CreateProxyRequest,
    request_timeout: float | None = None,
) -> CreateProxyResponse:
    payload = await transport.request(
        Method.POST, '/proxies',
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(CreateProxyResponse).validate_json(payload)

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.connection_list_response import ConnectionListResponse


async def list_connections(
    transport: Transport,
    *,
    request_timeout: float | None = None,
) -> ConnectionListResponse:
    payload = await transport.request(
        Method.GET, '/connections',
        timeout=request_timeout,
    )
    return TypeAdapter(ConnectionListResponse).validate_json(payload)

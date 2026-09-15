"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.network_list_response import NetworkListResponse


async def list_networks(
    transport: Transport,
) -> NetworkListResponse:
    payload = await transport.request(
        Method.GET, '/networks',
    )
    return TypeAdapter(NetworkListResponse).validate_json(payload)

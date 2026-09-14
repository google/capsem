"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.create_network_request import CreateNetworkRequest
from ..models.network_info import NetworkInfo


async def create_network(
    transport: Transport,
    *,
    body: CreateNetworkRequest,
) -> NetworkInfo:
    payload = await transport.request(
        Method.POST, '/networks',
        body=body,
    )
    return TypeAdapter(NetworkInfo).validate_json(payload)

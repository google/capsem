"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.network_info import NetworkInfo


async def get_network(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> NetworkInfo:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/networks/{id}',
        path_parameters={'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(NetworkInfo).validate_json(payload)

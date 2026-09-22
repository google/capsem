"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.network_info import NetworkInfo


async def attach_network_member(
    transport: Transport,
    *,
    id: StrictStr,
    vm_id: StrictStr,
    request_timeout: float | None = None,
) -> NetworkInfo:
    id = TypeAdapter(StrictStr).validate_python(id)
    vm_id = TypeAdapter(StrictStr).validate_python(vm_id)
    payload = await transport.request(
        Method.PUT, '/networks/{id}/members/{vm_id}',
        path_parameters={'id': id, 'vm_id': vm_id},
        timeout=request_timeout,
    )
    return TypeAdapter(NetworkInfo).validate_json(payload)

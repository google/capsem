"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.container_status_response import ContainerStatusResponse


async def get_vm_container(
    transport: Transport,
    *,
    id: StrictStr,
) -> ContainerStatusResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/container',
        path_parameters={'id': id},
    )
    return TypeAdapter(ContainerStatusResponse).validate_json(payload)

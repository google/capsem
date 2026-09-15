"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_status_response import VmStatusResponse


async def get_vm_status(
    transport: Transport,
    *,
    id: StrictStr,
) -> VmStatusResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/status',
        path_parameters={'id': id},
    )
    return TypeAdapter(VmStatusResponse).validate_json(payload)

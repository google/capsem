"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_action_response import VmActionResponse


async def delete_vm(
    transport: Transport,
    *,
    id: StrictStr,
) -> VmActionResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.DELETE, '/vms/{id}/delete',
        path_parameters={'id': id},
    )
    return TypeAdapter(VmActionResponse).validate_json(payload)

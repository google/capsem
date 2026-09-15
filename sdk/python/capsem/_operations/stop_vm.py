"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.stop_response import StopResponse


async def stop_vm(
    transport: Transport,
    *,
    id: StrictStr,
) -> StopResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/stop',
        path_parameters={'id': id},
    )
    return TypeAdapter(StopResponse).validate_json(payload)

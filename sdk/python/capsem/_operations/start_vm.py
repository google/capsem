"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.provision_response import ProvisionResponse


async def start_vm(
    transport: Transport,
    *,
    id: StrictStr,
) -> ProvisionResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/start',
        path_parameters={'id': id},
    )
    return TypeAdapter(ProvisionResponse).validate_json(payload)

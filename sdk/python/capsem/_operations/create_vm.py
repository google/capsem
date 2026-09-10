"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.provision_request import ProvisionRequest
from ..models.provision_response import ProvisionResponse


async def create_vm(
    transport: Transport,
    *,
    body: ProvisionRequest,
) -> ProvisionResponse:
    payload = await transport.request(
        Method.POST, '/vms/create',
        body=body,
    )
    return TypeAdapter(ProvisionResponse).validate_json(payload)

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.provision_response import ProvisionResponse
from ..models.vm_start_request import VmStartRequest


async def resume_vm(
    transport: Transport,
    *,
    id: StrictStr,
    body: VmStartRequest | None = None,
    request_timeout: float | None = None,
) -> ProvisionResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    body = TypeAdapter(VmStartRequest | None).validate_python(body)
    payload = await transport.request(
        Method.POST, '/vms/{id}/resume',
        path_parameters={'id': id},
        body=body,
        json_body=body is not None,
        timeout=request_timeout,
    )
    return TypeAdapter(ProvisionResponse).validate_json(payload)

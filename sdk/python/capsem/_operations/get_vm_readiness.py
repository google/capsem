"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_readiness_response import VmReadinessResponse


async def get_vm_readiness(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> VmReadinessResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/readiness',
        path_parameters={'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(VmReadinessResponse).validate_json(payload)

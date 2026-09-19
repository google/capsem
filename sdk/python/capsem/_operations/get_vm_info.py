"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.sandbox_info import SandboxInfo


async def get_vm_info(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> SandboxInfo:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/info',
        path_parameters={'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(SandboxInfo).validate_json(payload)

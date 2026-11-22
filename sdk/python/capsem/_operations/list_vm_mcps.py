"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_mcp_servers_response import VmMcpServersResponse


async def list_vm_mcps(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> VmMcpServersResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/mcps',
        path_parameters={'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(VmMcpServersResponse).validate_json(payload)

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_mcp_tools_response import VmMcpToolsResponse


async def list_vm_mcp_tools(
    transport: Transport,
    *,
    id: StrictStr,
    server_id: StrictStr,
    request_timeout: float | None = None,
) -> VmMcpToolsResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    server_id = TypeAdapter(StrictStr).validate_python(server_id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/mcps/{server_id}/tools',
        path_parameters={'id': id, 'server_id': server_id},
        timeout=request_timeout,
    )
    return TypeAdapter(VmMcpToolsResponse).validate_json(payload)

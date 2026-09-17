"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_tools_list_response import McpToolsListResponse


async def list_profile_mcp_tools(
    transport: Transport,
    *,
    profile_id: StrictStr,
    server_id: StrictStr,
    request_timeout: float | None = None,
) -> McpToolsListResponse:
    profile_id = TypeAdapter(StrictStr).validate_python(profile_id)
    server_id = TypeAdapter(StrictStr).validate_python(server_id)
    payload = await transport.request(
        Method.GET, '/profiles/{profile_id}/mcp/servers/{server_id}/tools/list',
        path_parameters={'profile_id': profile_id, 'server_id': server_id},
        timeout=request_timeout,
    )
    return TypeAdapter(McpToolsListResponse).validate_json(payload)

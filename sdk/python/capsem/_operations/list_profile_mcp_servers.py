"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_servers_list_response import McpServersListResponse


async def list_profile_mcp_servers(
    transport: Transport,
    *,
    profile_id: StrictStr,
    request_timeout: float | None = None,
) -> McpServersListResponse:
    profile_id = TypeAdapter(StrictStr).validate_python(profile_id)
    payload = await transport.request(
        Method.GET, '/profiles/{profile_id}/mcp/servers/list',
        path_parameters={'profile_id': profile_id},
        timeout=request_timeout,
    )
    return TypeAdapter(McpServersListResponse).validate_json(payload)

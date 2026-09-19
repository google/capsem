"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_refresh_response import McpRefreshResponse


async def refresh_profile_mcp_server(
    transport: Transport,
    *,
    profile_id: StrictStr,
    server_id: StrictStr,
    request_timeout: float | None = None,
) -> McpRefreshResponse:
    profile_id = TypeAdapter(StrictStr).validate_python(profile_id)
    server_id = TypeAdapter(StrictStr).validate_python(server_id)
    payload = await transport.request(
        Method.POST, '/profiles/{profile_id}/mcp/servers/{server_id}/refresh',
        path_parameters={'profile_id': profile_id, 'server_id': server_id},
        timeout=request_timeout,
    )
    return TypeAdapter(McpRefreshResponse).validate_json(payload)

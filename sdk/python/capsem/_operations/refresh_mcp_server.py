"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_refresh_response import McpRefreshResponse


async def refresh_mcp_server(
    transport: Transport,
    *,
    server_id: StrictStr,
    request_timeout: float | None = None,
) -> McpRefreshResponse:
    server_id = TypeAdapter(StrictStr).validate_python(server_id)
    payload = await transport.request(
        Method.POST, '/mcp/servers/{server_id}/refresh',
        path_parameters={'server_id': server_id},
        timeout=request_timeout,
    )
    return TypeAdapter(McpRefreshResponse).validate_json(payload)

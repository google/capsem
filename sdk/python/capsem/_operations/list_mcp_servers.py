"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_servers_list_response import McpServersListResponse


async def list_mcp_servers(
    transport: Transport,
    *,
    request_timeout: float | None = None,
) -> McpServersListResponse:
    payload = await transport.request(
        Method.GET, '/mcp/servers/list',
        timeout=request_timeout,
    )
    return TypeAdapter(McpServersListResponse).validate_json(payload)

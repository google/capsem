"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_default_permission_response import McpDefaultPermissionResponse


async def get_mcp_default(
    transport: Transport,
    *,
    request_timeout: float | None = None,
) -> McpDefaultPermissionResponse:
    payload = await transport.request(
        Method.GET, '/mcp/default/info',
        timeout=request_timeout,
    )
    return TypeAdapter(McpDefaultPermissionResponse).validate_json(payload)

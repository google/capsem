"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_info_response import McpInfoResponse


async def get_mcp_info(
    transport: Transport,
    *,
    request_timeout: float | None = None,
) -> McpInfoResponse:
    payload = await transport.request(
        Method.GET, '/mcp/info',
        timeout=request_timeout,
    )
    return TypeAdapter(McpInfoResponse).validate_json(payload)

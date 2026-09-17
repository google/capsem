"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_default_permission_response import McpDefaultPermissionResponse


async def get_profile_mcp_default(
    transport: Transport,
    *,
    profile_id: StrictStr,
    request_timeout: float | None = None,
) -> McpDefaultPermissionResponse:
    profile_id = TypeAdapter(StrictStr).validate_python(profile_id)
    payload = await transport.request(
        Method.GET, '/profiles/{profile_id}/mcp/default/info',
        path_parameters={'profile_id': profile_id},
        timeout=request_timeout,
    )
    return TypeAdapter(McpDefaultPermissionResponse).validate_json(payload)

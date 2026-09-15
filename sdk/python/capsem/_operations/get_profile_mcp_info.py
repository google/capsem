"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.profile_mcp_info_response import ProfileMcpInfoResponse


async def get_profile_mcp_info(
    transport: Transport,
    *,
    profile_id: StrictStr,
) -> ProfileMcpInfoResponse:
    profile_id = TypeAdapter(StrictStr).validate_python(profile_id)
    payload = await transport.request(
        Method.GET, '/profiles/{profile_id}/mcp/info',
        path_parameters={'profile_id': profile_id},
    )
    return TypeAdapter(ProfileMcpInfoResponse).validate_json(payload)

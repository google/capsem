"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.value import Value


async def call_profile_mcp_tool(
    transport: Transport,
    *,
    profile_id: StrictStr,
    server_id: StrictStr,
    tool_id: StrictStr,
    body: Value,
) -> Value:
    profile_id = TypeAdapter(StrictStr).validate_python(profile_id)
    server_id = TypeAdapter(StrictStr).validate_python(server_id)
    tool_id = TypeAdapter(StrictStr).validate_python(tool_id)
    payload = await transport.request(
        Method.POST, '/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call',
        path_parameters={'profile_id': profile_id, 'server_id': server_id, 'tool_id': tool_id},
        body=body,
        json_body=True,
    )
    return TypeAdapter(Value).validate_json(payload)

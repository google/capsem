"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.value import Value


async def call_mcp_tool(
    transport: Transport,
    *,
    server_id: StrictStr,
    tool_id: StrictStr,
    body: Value,
    request_timeout: float | None = None,
) -> Value:
    server_id = TypeAdapter(StrictStr).validate_python(server_id)
    tool_id = TypeAdapter(StrictStr).validate_python(tool_id)
    payload = await transport.request(
        Method.POST, '/mcp/servers/{server_id}/tools/{tool_id}/call',
        path_parameters={'server_id': server_id, 'tool_id': tool_id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(Value).validate_json(payload)

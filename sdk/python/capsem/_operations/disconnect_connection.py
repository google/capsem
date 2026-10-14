"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.disconnect_response import DisconnectResponse


async def disconnect_connection(
    transport: Transport,
    *,
    connection_id: StrictStr,
    request_timeout: float | None = None,
) -> DisconnectResponse:
    connection_id = TypeAdapter(StrictStr).validate_python(connection_id)
    payload = await transport.request(
        Method.DELETE, '/connections/{connection_id}',
        path_parameters={'connection_id': connection_id},
        timeout=request_timeout,
    )
    return TypeAdapter(DisconnectResponse).validate_json(payload)

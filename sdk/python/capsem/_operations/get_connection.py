"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.connection_summary import ConnectionSummary


async def get_connection(
    transport: Transport,
    *,
    connection_id: StrictStr,
    request_timeout: float | None = None,
) -> ConnectionSummary:
    connection_id = TypeAdapter(StrictStr).validate_python(connection_id)
    payload = await transport.request(
        Method.GET, '/connections/{connection_id}',
        path_parameters={'connection_id': connection_id},
        timeout=request_timeout,
    )
    return TypeAdapter(ConnectionSummary).validate_json(payload)

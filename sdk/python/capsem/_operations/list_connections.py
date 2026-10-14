"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, TypeAdapter

from .._transport import Method, Transport
from ..models.connection_list_response import ConnectionListResponse
from ..models.model_base import UuidString


async def list_connections(
    transport: Transport,
    *,
    limit: Annotated[StrictInt, Field(ge=1)] | None = None,
    cursor: UuidString | None = None,
    request_timeout: float | None = None,
) -> ConnectionListResponse:
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=1)] | None).validate_python(limit)
    cursor = TypeAdapter(UuidString | None).validate_python(cursor)
    payload = await transport.request(
        Method.GET, '/connections',
        query={'limit': limit, 'cursor': cursor},
        timeout=request_timeout,
    )
    return TypeAdapter(ConnectionListResponse).validate_json(payload)

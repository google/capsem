"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.list_response import ListResponse


async def list_vms(
    transport: Transport,
) -> ListResponse:
    payload = await transport.request(
        Method.GET, '/vms/list',
    )
    return TypeAdapter(ListResponse).validate_json(payload)

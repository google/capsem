"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.purge_request import PurgeRequest
from ..models.purge_response import PurgeResponse


async def purge_vms(
    transport: Transport,
    *,
    body: PurgeRequest,
) -> PurgeResponse:
    payload = await transport.request(
        Method.POST, '/purge',
        body=body,
        json_body=True,
    )
    return TypeAdapter(PurgeResponse).validate_json(payload)

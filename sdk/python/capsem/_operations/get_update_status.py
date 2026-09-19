"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.update_status_response import UpdateStatusResponse


async def get_update_status(
    transport: Transport,
    *,
    request_timeout: float | None = None,
) -> UpdateStatusResponse:
    payload = await transport.request(
        Method.GET, '/update/status',
        timeout=request_timeout,
    )
    return TypeAdapter(UpdateStatusResponse).validate_json(payload)

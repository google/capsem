"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.restart_response import RestartResponse


async def restart_hypervisor(
    transport: Transport,
) -> RestartResponse:
    payload = await transport.request(
        Method.POST, '/restart',
    )
    return TypeAdapter(RestartResponse).validate_json(payload)

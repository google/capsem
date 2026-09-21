"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.hypervisor_info import HypervisorInfo


async def get_hypervisor_info(
    transport: Transport,
    *,
    request_timeout: float | None = None,
) -> HypervisorInfo:
    payload = await transport.request(
        Method.GET, '/status',
        timeout=request_timeout,
    )
    return TypeAdapter(HypervisorInfo).validate_json(payload)

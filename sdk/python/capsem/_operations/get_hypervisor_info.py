"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.hypervisor_info import HypervisorInfo


async def get_hypervisor_info(
    transport: Transport,
) -> HypervisorInfo:
    payload = await transport.request(
        Method.GET, '/status',
    )
    return TypeAdapter(HypervisorInfo).validate_json(payload)

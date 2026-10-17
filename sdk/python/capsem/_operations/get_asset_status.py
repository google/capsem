"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.asset_status import AssetStatus


async def get_asset_status(
    transport: Transport,
    *,
    request_timeout: float | None = None,
) -> AssetStatus:
    payload = await transport.request(
        Method.GET, '/assets/status',
        timeout=request_timeout,
    )
    return TypeAdapter(AssetStatus).validate_json(payload)

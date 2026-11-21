"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.request_detail_response import RequestDetailResponse


async def get_request_info(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> RequestDetailResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/requests/{id}/info',
        path_parameters={'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(RequestDetailResponse).validate_json(payload)

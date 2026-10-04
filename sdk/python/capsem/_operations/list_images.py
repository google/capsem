"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, TypeAdapter

from .._transport import Method, Transport
from ..models.image_list_response import ImageListResponse


async def list_images(
    transport: Transport,
    *,
    refresh: StrictBool | None = None,
    request_timeout: float | None = None,
) -> ImageListResponse:
    refresh = TypeAdapter(StrictBool | None).validate_python(refresh)
    payload = await transport.request(
        Method.GET, '/images',
        query={'refresh': refresh},
        timeout=request_timeout,
    )
    return TypeAdapter(ImageListResponse).validate_json(payload)

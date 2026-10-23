"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.image_inventory_response import ImageInventoryResponse


async def get_image_inventory(
    transport: Transport,
    *,
    name: StrictStr,
    image: StrictStr,
    request_timeout: float | None = None,
) -> ImageInventoryResponse:
    name = TypeAdapter(StrictStr).validate_python(name)
    image = TypeAdapter(StrictStr).validate_python(image)
    payload = await transport.request(
        Method.GET, '/images/{name}/inventory',
        path_parameters={'name': name},
        query={'image': image},
        timeout=request_timeout,
    )
    return TypeAdapter(ImageInventoryResponse).validate_json(payload)

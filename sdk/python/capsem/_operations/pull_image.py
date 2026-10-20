"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.image_pull_request import ImagePullRequest
from ..models.image_pull_response import ImagePullResponse


async def pull_image(
    transport: Transport,
    *,
    body: ImagePullRequest,
    request_timeout: float | None = None,
) -> ImagePullResponse:
    payload = await transport.request(
        Method.POST, '/images/pull',
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(ImagePullResponse).validate_json(payload)

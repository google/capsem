"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_image_update_response import EnvironmentImageUpdateResponse


async def check_environment_image_update(
    transport: Transport,
    *,
    id: StrictStr,
    revision_id: StrictStr,
    request_timeout: float | None = None,
) -> EnvironmentImageUpdateResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    revision_id = TypeAdapter(StrictStr).validate_python(revision_id)
    payload = await transport.request(
        Method.GET, '/environments/{id}/revisions/{revision_id}/image-update',
        path_parameters={'id': id, 'revision_id': revision_id},
        timeout=request_timeout if request_timeout is not None else max(transport.timeout, 1020.0),
    )
    return TypeAdapter(EnvironmentImageUpdateResponse).validate_json(payload)

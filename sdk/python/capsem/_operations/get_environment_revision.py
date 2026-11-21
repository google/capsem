"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_revision_detail_response import EnvironmentRevisionDetailResponse


async def get_environment_revision(
    transport: Transport,
    *,
    id: StrictStr,
    revision_id: StrictStr,
    request_timeout: float | None = None,
) -> EnvironmentRevisionDetailResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    revision_id = TypeAdapter(StrictStr).validate_python(revision_id)
    payload = await transport.request(
        Method.GET, '/environments/{id}/revisions/{revision_id}/info',
        path_parameters={'id': id, 'revision_id': revision_id},
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentRevisionDetailResponse).validate_json(payload)

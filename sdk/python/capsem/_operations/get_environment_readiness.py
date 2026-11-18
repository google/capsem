"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_readiness_response import EnvironmentReadinessResponse


async def get_environment_readiness(
    transport: Transport,
    *,
    id: StrictStr,
    revision_id: StrictStr,
    request_timeout: float | None = None,
) -> EnvironmentReadinessResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    revision_id = TypeAdapter(StrictStr).validate_python(revision_id)
    payload = await transport.request(
        Method.GET, '/environments/{id}/revisions/{revision_id}/readiness',
        path_parameters={'id': id, 'revision_id': revision_id},
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentReadinessResponse).validate_json(payload)

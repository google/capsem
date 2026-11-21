"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_revision_list_response import EnvironmentRevisionListResponse


async def list_environment_revisions(
    transport: Transport,
    *,
    id: StrictStr,
    before: StrictInt | None = None,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    request_timeout: float | None = None,
) -> EnvironmentRevisionListResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    before = TypeAdapter(StrictInt | None).validate_python(before)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    payload = await transport.request(
        Method.GET, '/environments/{id}/revisions/list',
        path_parameters={'id': id},
        query={'before': before, 'limit': limit},
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentRevisionListResponse).validate_json(payload)

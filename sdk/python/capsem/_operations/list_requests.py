"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, TypeAdapter

from .._transport import Method, Transport
from ..models.request_list_response import RequestListResponse


async def list_requests(
    transport: Transport,
    *,
    after: StrictInt | None = None,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    pending_only: StrictBool | None = None,
    request_timeout: float | None = None,
) -> RequestListResponse:
    after = TypeAdapter(StrictInt | None).validate_python(after)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    pending_only = TypeAdapter(StrictBool | None).validate_python(pending_only)
    payload = await transport.request(
        Method.GET, '/requests/list',
        query={'after': after, 'limit': limit, 'pending_only': pending_only},
        timeout=request_timeout,
    )
    return TypeAdapter(RequestListResponse).validate_json(payload)

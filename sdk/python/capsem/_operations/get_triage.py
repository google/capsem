"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.triage_response import TriageResponse


async def get_triage(
    transport: Transport,
    *,
    since: StrictStr | None = None,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    id: StrictStr | None = None,
    request_timeout: float | None = None,
) -> TriageResponse:
    since = TypeAdapter(StrictStr | None).validate_python(since)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    id = TypeAdapter(StrictStr | None).validate_python(id)
    payload = await transport.request(
        Method.GET, '/triage',
        query={'since': since, 'limit': limit, 'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(TriageResponse).validate_json(payload)

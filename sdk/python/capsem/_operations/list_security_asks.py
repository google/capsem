"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.security_ask_list_response import SecurityAskListResponse


async def list_security_asks(
    transport: Transport,
    *,
    id: StrictStr,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    request_timeout: float | None = None,
) -> SecurityAskListResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    payload = await transport.request(
        Method.GET, '/vms/{id}/security/asks',
        path_parameters={'id': id},
        query={'limit': limit},
        timeout=request_timeout,
    )
    return TypeAdapter(SecurityAskListResponse).validate_json(payload)

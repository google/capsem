"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.logs_response import LogsResponse


async def get_vm_logs(
    transport: Transport,
    *,
    id: StrictStr,
    grep: StrictStr | None = None,
    tail: Annotated[StrictInt, Field(ge=0)] | None = None,
    max_bytes: Annotated[StrictInt, Field(ge=0)] | None = None,
) -> LogsResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    grep = TypeAdapter(StrictStr | None).validate_python(grep)
    tail = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(tail)
    max_bytes = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(max_bytes)
    payload = await transport.request(
        Method.GET, '/vms/{id}/logs',
        path_parameters={'id': id},
        query={'grep': grep, 'tail': tail, 'max_bytes': max_bytes},
    )
    return TypeAdapter(LogsResponse).validate_json(payload)

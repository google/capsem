"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.host_log_source import HostLogSource
from ..models.host_logs_response import HostLogsResponse


async def get_hypervisor_logs(
    transport: Transport,
    *,
    name: HostLogSource,
    grep: StrictStr | None = None,
    tail: Annotated[StrictInt, Field(ge=0)] | None = None,
    max_bytes: Annotated[StrictInt, Field(ge=0)] | None = None,
) -> HostLogsResponse:
    name = TypeAdapter(HostLogSource).validate_python(name)
    grep = TypeAdapter(StrictStr | None).validate_python(grep)
    tail = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(tail)
    max_bytes = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(max_bytes)
    payload = await transport.request(
        Method.GET, '/host-logs/{name}',
        path_parameters={'name': name},
        query={'grep': grep, 'tail': tail, 'max_bytes': max_bytes},
    )
    return TypeAdapter(HostLogsResponse).validate_json(payload)

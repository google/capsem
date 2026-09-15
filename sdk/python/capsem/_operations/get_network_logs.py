"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.network_logs_response import NetworkLogsResponse


async def get_network_logs(
    transport: Transport,
    *,
    id: StrictStr,
    cursor: StrictStr | None = None,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    vm: StrictStr | None = None,
    connection: StrictStr | None = None,
    type: StrictStr | None = None,
    decision: StrictStr | None = None,
    since: StrictInt | None = None,
    until: StrictInt | None = None,
) -> NetworkLogsResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    cursor = TypeAdapter(StrictStr | None).validate_python(cursor)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    vm = TypeAdapter(StrictStr | None).validate_python(vm)
    connection = TypeAdapter(StrictStr | None).validate_python(connection)
    type = TypeAdapter(StrictStr | None).validate_python(type)
    decision = TypeAdapter(StrictStr | None).validate_python(decision)
    since = TypeAdapter(StrictInt | None).validate_python(since)
    until = TypeAdapter(StrictInt | None).validate_python(until)
    payload = await transport.request(
        Method.GET, '/networks/{id}/logs',
        path_parameters={'id': id},
        query={'cursor': cursor, 'limit': limit, 'vm': vm, 'connection': connection, 'type': type, 'decision': decision, 'since': since, 'until': until},
    )
    return TypeAdapter(NetworkLogsResponse).validate_json(payload)

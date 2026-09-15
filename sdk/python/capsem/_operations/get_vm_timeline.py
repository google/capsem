"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.timeline_layer import TimelineLayer
from ..models.timeline_response import TimelineResponse


async def get_vm_timeline(
    transport: Transport,
    *,
    id: StrictStr,
    trace_id: StrictStr | None = None,
    since: StrictStr | None = None,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    layers: list[TimelineLayer] | None = None,
) -> TimelineResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    trace_id = TypeAdapter(StrictStr | None).validate_python(trace_id)
    since = TypeAdapter(StrictStr | None).validate_python(since)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    layers = TypeAdapter(list[TimelineLayer] | None).validate_python(layers)
    payload = await transport.request(
        Method.GET, '/vms/{id}/timeline',
        path_parameters={'id': id},
        query={'trace_id': trace_id, 'since': since, 'limit': limit, 'layers': layers},
    )
    return TypeAdapter(TimelineResponse).validate_json(payload)

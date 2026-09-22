"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.event_bodies_response import EventBodiesResponse


async def get_vm_event_bodies(
    transport: Transport,
    *,
    id: StrictStr,
    event_id: StrictStr,
    max_bytes: Annotated[StrictInt, Field(ge=0)] | None = None,
    request_timeout: float | None = None,
) -> EventBodiesResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    event_id = TypeAdapter(StrictStr).validate_python(event_id)
    max_bytes = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(max_bytes)
    payload = await transport.request(
        Method.GET, '/vms/{id}/bodies/{event_id}',
        path_parameters={'id': id, 'event_id': event_id},
        query={'max_bytes': max_bytes},
        timeout=request_timeout,
    )
    return TypeAdapter(EventBodiesResponse).validate_json(payload)

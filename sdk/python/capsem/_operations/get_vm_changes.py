"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.changes_response import ChangesResponse


async def get_vm_changes(
    transport: Transport,
    *,
    id: StrictStr,
    checkpoint: StrictStr,
    offset: Annotated[StrictInt, Field(ge=0)] | None = None,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
) -> ChangesResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    checkpoint = TypeAdapter(StrictStr).validate_python(checkpoint)
    offset = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(offset)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    payload = await transport.request(
        Method.GET, '/vms/{id}/changes',
        path_parameters={'id': id},
        query={'checkpoint': checkpoint, 'offset': offset, 'limit': limit},
    )
    return TypeAdapter(ChangesResponse).validate_json(payload)

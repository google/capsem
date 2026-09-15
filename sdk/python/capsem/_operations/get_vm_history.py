"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.history_layer_filter import HistoryLayerFilter
from ..models.history_response import HistoryResponse


async def get_vm_history(
    transport: Transport,
    *,
    id: StrictStr,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    offset: Annotated[StrictInt, Field(ge=0)] | None = None,
    search: StrictStr | None = None,
    layer: HistoryLayerFilter | None = None,
) -> HistoryResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    offset = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(offset)
    search = TypeAdapter(StrictStr | None).validate_python(search)
    layer = TypeAdapter(HistoryLayerFilter | None).validate_python(layer)
    payload = await transport.request(
        Method.GET, '/vms/{id}/history',
        path_parameters={'id': id},
        query={'limit': limit, 'offset': offset, 'search': search, 'layer': layer},
    )
    return TypeAdapter(HistoryResponse).validate_json(payload)

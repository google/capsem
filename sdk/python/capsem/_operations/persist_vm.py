"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.persist_request import PersistRequest
from ..models.persist_response import PersistResponse


async def persist_vm(
    transport: Transport,
    *,
    id: StrictStr,
    body: PersistRequest,
    request_timeout: float | None = None,
) -> PersistResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/save',
        path_parameters={'id': id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(PersistResponse).validate_json(payload)

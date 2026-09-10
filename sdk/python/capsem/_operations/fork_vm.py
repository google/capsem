"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.fork_request import ForkRequest
from ..models.fork_response import ForkResponse


async def fork_vm(
    transport: Transport,
    *,
    id: StrictStr,
    body: ForkRequest,
) -> ForkResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/fork',
        path_parameters={'id': id},
        body=body,
    )
    return TypeAdapter(ForkResponse).validate_json(payload)

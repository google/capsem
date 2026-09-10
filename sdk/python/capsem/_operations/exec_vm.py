"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.exec_request import ExecRequest
from ..models.exec_response import ExecResponse


async def exec_vm(
    transport: Transport,
    *,
    id: StrictStr,
    body: ExecRequest,
) -> ExecResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/exec',
        path_parameters={'id': id},
        body=body,
    )
    return TypeAdapter(ExecResponse).validate_json(payload)

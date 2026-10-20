"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.managed_session_close_request import ManagedSessionCloseRequest
from ..models.managed_session_close_response import ManagedSessionCloseResponse


async def close_managed_session(
    transport: Transport,
    *,
    request_id: StrictStr,
    body: ManagedSessionCloseRequest,
    request_timeout: float | None = None,
) -> ManagedSessionCloseResponse:
    request_id = TypeAdapter(StrictStr).validate_python(request_id)
    payload = await transport.request(
        Method.POST, '/vms/managed/{request_id}/close',
        path_parameters={'request_id': request_id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(ManagedSessionCloseResponse).validate_json(payload)

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.session_grant_list_response import SessionGrantListResponse


async def list_session_credential_grants(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> SessionGrantListResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/credential-grants',
        path_parameters={'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(SessionGrantListResponse).validate_json(payload)

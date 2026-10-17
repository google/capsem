"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.create_session_grant_request import CreateSessionGrantRequest
from ..models.session_grant_summary import SessionGrantSummary


async def create_session_credential_grant(
    transport: Transport,
    *,
    id: StrictStr,
    body: CreateSessionGrantRequest,
    request_timeout: float | None = None,
) -> SessionGrantSummary:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/credential-grants',
        path_parameters={'id': id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(SessionGrantSummary).validate_json(payload)

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.revoke_session_grant_response import RevokeSessionGrantResponse


async def revoke_session_credential_grant(
    transport: Transport,
    *,
    id: StrictStr,
    grant_id: StrictStr,
    request_timeout: float | None = None,
) -> RevokeSessionGrantResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    grant_id = TypeAdapter(StrictStr).validate_python(grant_id)
    payload = await transport.request(
        Method.DELETE, '/vms/{id}/credential-grants/{grant_id}',
        path_parameters={'id': id, 'grant_id': grant_id},
        timeout=request_timeout,
    )
    return TypeAdapter(RevokeSessionGrantResponse).validate_json(payload)

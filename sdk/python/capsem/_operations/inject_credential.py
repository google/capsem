"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.credential_inject_request import CredentialInjectRequest
from ..models.credential_inject_response import CredentialInjectResponse


async def inject_credential(
    transport: Transport,
    *,
    body: CredentialInjectRequest,
    request_timeout: float | None = None,
) -> CredentialInjectResponse:
    payload = await transport.request(
        Method.POST, '/credentials/inject',
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(CredentialInjectResponse).validate_json(payload)

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.authorize_google_request import AuthorizeGoogleRequest
from ..models.authorize_google_response import AuthorizeGoogleResponse


async def authorize_google_connection(
    transport: Transport,
    *,
    body: AuthorizeGoogleRequest,
    request_timeout: float | None = None,
) -> AuthorizeGoogleResponse:
    payload = await transport.request(
        Method.POST, '/connections/google/authorize',
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(AuthorizeGoogleResponse).validate_json(payload)

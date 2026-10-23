"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.security_ask_request import SecurityAskRequest
from ..models.security_ask_resolve_request import SecurityAskResolveRequest


async def resolve_security_ask(
    transport: Transport,
    *,
    id: StrictStr,
    ask_id: StrictStr,
    body: SecurityAskResolveRequest,
    request_timeout: float | None = None,
) -> SecurityAskRequest:
    id = TypeAdapter(StrictStr).validate_python(id)
    ask_id = TypeAdapter(StrictStr).validate_python(ask_id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/security/asks/{ask_id}/resolve',
        path_parameters={'id': id, 'ask_id': ask_id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(SecurityAskRequest).validate_json(payload)

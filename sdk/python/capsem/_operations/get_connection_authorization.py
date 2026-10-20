"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.authorization_transaction import AuthorizationTransaction


async def get_connection_authorization(
    transport: Transport,
    *,
    transaction_id: StrictStr,
    request_timeout: float | None = None,
) -> AuthorizationTransaction:
    transaction_id = TypeAdapter(StrictStr).validate_python(transaction_id)
    payload = await transport.request(
        Method.GET, '/connections/authorizations/{transaction_id}',
        path_parameters={'transaction_id': transaction_id},
        timeout=request_timeout,
    )
    return TypeAdapter(AuthorizationTransaction).validate_json(payload)

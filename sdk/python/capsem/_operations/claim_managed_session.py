"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.managed_session_claim_request import ManagedSessionClaimRequest
from ..models.managed_session_claim_response import ManagedSessionClaimResponse


async def claim_managed_session(
    transport: Transport,
    *,
    request_id: StrictStr,
    body: ManagedSessionClaimRequest,
    request_timeout: float | None = None,
) -> ManagedSessionClaimResponse:
    request_id = TypeAdapter(StrictStr).validate_python(request_id)
    payload = await transport.request(
        Method.POST, '/vms/managed/{request_id}/claim',
        path_parameters={'request_id': request_id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(ManagedSessionClaimResponse).validate_json(payload)

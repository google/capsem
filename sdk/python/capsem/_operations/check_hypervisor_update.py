"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.update_action_response import UpdateActionResponse
from ..models.update_check_request import UpdateCheckRequest


async def check_hypervisor_update(
    transport: Transport,
    *,
    body: UpdateCheckRequest,
    request_timeout: float | None = None,
) -> UpdateActionResponse:
    payload = await transport.request(
        Method.POST, '/update/check',
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(UpdateActionResponse).validate_json(payload)

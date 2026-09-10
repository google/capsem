"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.update_action_response import UpdateActionResponse
from ..models.update_apply_request import UpdateApplyRequest


async def update_hypervisor(
    transport: Transport,
    *,
    body: UpdateApplyRequest,
) -> UpdateActionResponse:
    payload = await transport.request(
        Method.POST, '/update/apply',
        body=body,
    )
    return TypeAdapter(UpdateActionResponse).validate_json(payload)

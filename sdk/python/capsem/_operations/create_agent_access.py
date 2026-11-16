"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.create_agent_access_request import CreateAgentAccessRequest
from ..models.create_agent_access_response import CreateAgentAccessResponse


async def create_agent_access(
    transport: Transport,
    *,
    body: CreateAgentAccessRequest,
    request_timeout: float | None = None,
) -> CreateAgentAccessResponse:
    payload = await transport.request(
        Method.POST, '/agent-access',
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(CreateAgentAccessResponse).validate_json(payload)

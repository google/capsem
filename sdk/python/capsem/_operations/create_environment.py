"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.environment_create_request import EnvironmentCreateRequest
from ..models.environment_mutation_response import EnvironmentMutationResponse


async def create_environment(
    transport: Transport,
    *,
    body: EnvironmentCreateRequest,
    request_timeout: float | None = None,
) -> EnvironmentMutationResponse:
    payload = await transport.request(
        Method.POST, '/environments/create',
        body=body,
        json_body=True,
        timeout=request_timeout if request_timeout is not None else max(transport.timeout, 1020.0),
    )
    return TypeAdapter(EnvironmentMutationResponse).validate_json(payload)
